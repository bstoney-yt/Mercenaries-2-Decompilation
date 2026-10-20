//! The connection to the other player, run on a thread of its own so
//! the game's frame never waits on the network.
//!
//! Built on iroh: QUIC between the two games, dialled by the host's
//! public key. It punches through both players' routers when it can (no
//! port forwarding) and otherwise passes the packets through a relay
//! server, encrypted end to end either way.
//!
//! The game sends commands ([`Session::send_hero`], [`Session::leave`])
//! and drains [`Event`]s once a frame with [`Session::poll`].

use std::sync::mpsc;
use std::time::{Duration, Instant};

use iroh::endpoint::{Connection, RecvStream, SendStream, presets};
use iroh::{Endpoint, RelayMode, SecretKey};
use tokio::sync::mpsc as tmpsc;

use crate::invite::Invite;
use crate::proto::{Hello, HeroState, Msg, PeopleBatch, VERSION};

/// The protocol the two games agree on in the QUIC handshake.
pub const ALPN: &[u8] = b"m2-alpha/coop/1";

/// How long the host waits to reach its relay before inviting anyway
/// (with only its direct addresses).
const ONLINE_WAIT: Duration = Duration::from_secs(10);
/// How long joining may take before giving up.
const JOIN_WAIT: Duration = Duration::from_secs(30);
/// How often the link's state is reported.
const LINK_EVERY: Duration = Duration::from_secs(1);

/// What the session tells the game.
#[derive(Clone, Debug)]
pub enum Event {
    /// Hosting: the invite to send (once the host is reachable).
    Invite(Invite),
    /// The other player is in.
    Connected(Hello),
    /// The other player's hero, and when it arrived ([`Session::now`]).
    Hero(HeroState, f64),
    /// The host's people's places, and when they arrived.
    People(PeopleBatch, f64),
    /// Any other message from the other game (on the reliable stream).
    Message(Msg),
    /// How the link is doing.
    Link {
        rtt: Duration,
        /// Through the relay server rather than straight between the two.
        relayed: bool,
    },
    /// The other player left or was lost; hosting carries on waiting.
    Disconnected(String),
    /// The session could not start or carry on.
    Failed(String),
}

/// How the session reaches the other game.
#[derive(Clone, Debug)]
pub struct Options {
    /// Who this player is.
    pub me: Hello,
    /// Use the public relay and address lookup (the internet). Off, only
    /// direct addresses are used: a game on this machine or network.
    pub relay: bool,
    /// The host's key, kept so the same invite works next time. `None`:
    /// a fresh one.
    pub secret: Option<[u8; 32]>,
    /// Never straight between the two: every packet goes through the
    /// relay (for measuring the relay's path).
    pub relay_only: bool,
}

#[derive(Debug)]
enum Cmd {
    Hero(HeroState),
    /// Any other message: on the reliable stream, or as a datagram.
    Send(Msg, bool),
    Leave,
}

#[derive(Debug)]
pub struct Session {
    cmds: tmpsc::UnboundedSender<Cmd>,
    events: mpsc::Receiver<Event>,
    start: Instant,
}

impl Session {
    /// Hosts a game: waits for one friend at a time.
    pub fn host(options: Options) -> Self {
        Self::spawn(options, None)
    }

    /// Joins the game the invite names.
    pub fn join(invite: Invite, options: Options) -> Self {
        Self::spawn(options, Some(invite))
    }

    fn spawn(options: Options, invite: Option<Invite>) -> Self {
        let (cmds, cmd_rx) = tmpsc::unbounded_channel();
        let (tx, events) = mpsc::channel();
        let start = Instant::now();
        std::thread::Builder::new()
            .name("coop".into())
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = tx.send(Event::Failed(format!("network runtime: {e}")));
                        return;
                    }
                };
                let out = Out { tx, start };
                rt.block_on(async move {
                    let result = match invite {
                        None => host(options, cmd_rx, &out).await,
                        Some(i) => join(options, i, cmd_rx, &out).await,
                    };
                    if let Err(e) = result {
                        out.send(Event::Failed(e));
                    }
                });
            })
            .expect("spawn the co-op thread");
        Self {
            cmds,
            events,
            start,
        }
    }

    /// Seconds since the session started: the clock snapshots are taken
    /// and played back on.
    pub fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }

    /// Sends this player's hero (dropped if no one is connected).
    pub fn send_hero(&self, state: HeroState) {
        let _ = self.cmds.send(Cmd::Hero(state));
    }

    /// Sends `msg` in order and for sure (`reliable`), or as a datagram
    /// that may be lost. Dropped if no one is connected.
    pub fn send(&self, msg: Msg, reliable: bool) {
        let _ = self.cmds.send(Cmd::Send(msg, reliable));
    }

    /// Says goodbye to the other player and stops.
    pub fn leave(&self) {
        let _ = self.cmds.send(Cmd::Leave);
    }

    /// What happened since the last poll.
    pub fn poll(&self) -> Vec<Event> {
        self.events.try_iter().collect()
    }

    /// Waits up to `timeout` for the next event (for tools and tests).
    pub fn wait(&self, timeout: Duration) -> Option<Event> {
        self.events.recv_timeout(timeout).ok()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.leave();
    }
}

struct Out {
    tx: mpsc::Sender<Event>,
    start: Instant,
}

impl Out {
    fn send(&self, e: Event) {
        let _ = self.tx.send(e);
    }
    fn now(&self) -> f64 {
        self.start.elapsed().as_secs_f64()
    }
}

async fn bind(options: &Options) -> Result<Endpoint, String> {
    let secret = options
        .secret
        .map_or_else(SecretKey::generate, |b| SecretKey::from_bytes(&b));
    let builder = if options.relay {
        Endpoint::builder(presets::N0)
    } else {
        Endpoint::builder(presets::Minimal).relay_mode(RelayMode::Disabled)
    };
    let builder = if options.relay_only {
        builder.clear_ip_transports()
    } else {
        builder
    };
    builder
        .secret_key(secret)
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .map_err(|e| format!("could not open the network: {e}"))
}

async fn host(
    options: Options,
    mut cmds: tmpsc::UnboundedReceiver<Cmd>,
    out: &Out,
) -> Result<(), String> {
    let endpoint = bind(&options).await?;
    if options.relay
        && tokio::time::timeout(ONLINE_WAIT, endpoint.online())
            .await
            .is_err()
    {
        eprintln!("coop: the relay was not reached; inviting with direct addresses only");
    }
    out.send(Event::Invite(Invite::from_addr(&endpoint.addr())));
    loop {
        let incoming = tokio::select! {
            i = endpoint.accept() => match i {
                Some(i) => i,
                None => return Ok(()),
            },
            cmd = cmds.recv() => match cmd {
                None | Some(Cmd::Leave) => break,
                Some(Cmd::Hero(_) | Cmd::Send(..)) => continue,
            },
        };
        let conn = match incoming.accept() {
            Ok(a) => match a.await {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("coop: a connection failed: {e}");
                    continue;
                }
            },
            Err(e) => {
                eprintln!("coop: a connection failed: {e}");
                continue;
            }
        };
        let welcomed = async {
            let (mut send, mut recv) = conn.accept_bi().await.map_err(|e| e.to_string())?;
            let hello = match read_frame(&mut recv).await? {
                Msg::Hello(h) => h,
                other => return Err(format!("expected a hello, got {other:?}")),
            };
            if hello.version != VERSION {
                let why = format!(
                    "different versions (host {VERSION}, yours {}): both need the same beta",
                    hello.version
                );
                let _ = write_frame(&mut send, &Msg::Bye(why.clone())).await;
                return Err(why);
            }
            write_frame(&mut send, &Msg::Welcome(options.me.clone())).await?;
            Ok((send, recv, hello))
        };
        match tokio::time::timeout(JOIN_WAIT, welcomed).await {
            Ok(Ok((send, recv, hello))) => {
                out.send(Event::Connected(hello));
                if !play(&conn, send, recv, &mut cmds, out).await {
                    break;
                }
            }
            Ok(Err(e)) => eprintln!("coop: a friend could not join: {e}"),
            Err(_) => eprintln!("coop: a friend took too long to say hello"),
        }
    }
    endpoint.close().await;
    Ok(())
}

async fn join(
    options: Options,
    invite: Invite,
    mut cmds: tmpsc::UnboundedReceiver<Cmd>,
    out: &Out,
) -> Result<(), String> {
    let endpoint = bind(&options).await?;
    let joined = async {
        let conn = endpoint
            .connect(invite.to_addr(), ALPN)
            .await
            .map_err(|e| format!("could not reach the host: {e}"))?;
        let (mut send, mut recv) = conn.open_bi().await.map_err(|e| e.to_string())?;
        write_frame(&mut send, &Msg::Hello(options.me.clone())).await?;
        match read_frame(&mut recv).await? {
            Msg::Welcome(h) => Ok((conn, send, recv, h)),
            Msg::Bye(why) => Err(format!("the host said no: {why}")),
            other => Err(format!("expected a welcome, got {other:?}")),
        }
    };
    let (conn, send, recv, host) = tokio::time::timeout(JOIN_WAIT, joined)
        .await
        .map_err(|_| "the host did not answer in time".to_string())??;
    out.send(Event::Connected(host));
    play(&conn, send, recv, &mut cmds, out).await;
    endpoint.close().await;
    Ok(())
}

/// Plays with one connected friend until either leaves. `false` when
/// this player left (hosting stops too).
async fn play(
    conn: &Connection,
    mut send: SendStream,
    recv: RecvStream,
    cmds: &mut tmpsc::UnboundedReceiver<Cmd>,
    out: &Out,
) -> bool {
    // Frames are read on a task of their own: a read cut short by the
    // select below would lose half a frame.
    let (frame_tx, mut frames) = tmpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        let mut recv = recv;
        loop {
            let f = read_frame(&mut recv).await;
            let end = f.is_err();
            if frame_tx.send(f).is_err() || end {
                break;
            }
        }
    });
    let mut link = tokio::time::interval(LINK_EVERY);
    let mut seq_out = 0u32;
    let stayed = loop {
        tokio::select! {
            d = conn.read_datagram() => match d {
                Ok(bytes) => match Msg::decode(&bytes) {
                    Ok(Msg::Hero(s)) => out.send(Event::Hero(s, out.now())),
                    Ok(Msg::People(b)) => out.send(Event::People(b, out.now())),
                    Ok(other) => eprintln!("coop: unexpected datagram {other:?}"),
                    Err(e) => eprintln!("coop: bad datagram: {e}"),
                },
                Err(e) => {
                    out.send(Event::Disconnected(format!("lost the connection: {e}")));
                    break true;
                }
            },
            f = frames.recv() => match f {
                Some(Ok(Msg::Bye(why))) => {
                    out.send(Event::Disconnected(why));
                    break true;
                }
                Some(Ok(other)) => out.send(Event::Message(other)),
                Some(Err(e)) => {
                    out.send(Event::Disconnected(e));
                    break true;
                }
                None => {
                    out.send(Event::Disconnected("the other game closed".into()));
                    break true;
                }
            },
            cmd = cmds.recv() => match cmd {
                Some(Cmd::Hero(mut s)) => {
                    // The session numbers them, so a resumed game's
                    // count never runs backwards.
                    seq_out += 1;
                    s.seq = seq_out;
                    if let Err(e) = conn.send_datagram(Msg::Hero(s).encode().into()) {
                        eprintln!("coop: could not send: {e}");
                    }
                }
                Some(Cmd::Send(msg, true)) => {
                    if let Err(e) = write_frame(&mut send, &msg).await {
                        out.send(Event::Disconnected(format!("lost the connection: {e}")));
                        break true;
                    }
                }
                Some(Cmd::Send(msg, false)) => {
                    if let Err(e) = conn.send_datagram(msg.encode().into()) {
                        eprintln!("coop: could not send: {e}");
                    }
                }
                None | Some(Cmd::Leave) => {
                    let _ = write_frame(&mut send, &Msg::Bye("the other player left".into())).await;
                    let _ = send.finish();
                    // Give the goodbye a moment to go out.
                    let _ = tokio::time::timeout(Duration::from_millis(500), send.stopped()).await;
                    break false;
                }
            },
            _ = link.tick() => {
                let paths = conn.paths();
                if let Some(p) = paths.iter().find(|p| p.is_selected()) {
                    out.send(Event::Link { rtt: p.rtt(), relayed: p.is_relay() });
                }
            },
        }
    };
    reader.abort();
    conn.close(0u32.into(), b"bye");
    stayed
}

/// A reliable message: its length (`u16`, little-endian), then it.
async fn write_frame(send: &mut SendStream, msg: &Msg) -> Result<(), String> {
    let bytes = msg.encode();
    let mut framed = (bytes.len() as u16).to_le_bytes().to_vec();
    framed.extend_from_slice(&bytes);
    send.write_all(&framed).await.map_err(|e| e.to_string())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Msg, String> {
    let mut len = [0; 2];
    recv.read_exact(&mut len)
        .await
        .map_err(|e| format!("the other game closed ({e})"))?;
    let mut buf = vec![0; u16::from_le_bytes(len) as usize];
    recv.read_exact(&mut buf).await.map_err(|e| e.to_string())?;
    Msg::decode(&buf).map_err(|e| e.to_string())
}
