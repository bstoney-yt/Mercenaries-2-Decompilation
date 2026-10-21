//! A stand-in for the other player, with no game: hosts or joins, walks
//! a hero round a circle and logs what comes back. For testing the
//! connection and the alpha's co-op without a second copy of the game.
//!
//! ```text
//! coop_peer --host [--local] [--seconds 30] [--at x,y,z]
//! coop_peer --join <code or link> [--local] [--seconds 30] [--at x,y,z]
//! ```
//!
//! `--local` keeps to direct addresses (no relay). Prints `invite:` when
//! hosting, then a `peer:` line each second: snapshots received, the
//! link's round trip and whether it goes through the relay.

use std::time::{Duration, Instant};

use m2_net::{Event, Gait, Hello, HeroState, Invite, Options, Session};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let seconds: f64 = value("--seconds").map_or(30.0, |s| s.parse().expect("--seconds"));
    // Round this point, or round the other hero where it first is.
    let mut centre: Option<[f32; 3]> = value("--at").map(|s| {
        let v: Vec<f32> = s
            .split(',')
            .map(|v| v.parse().expect("--at x,y,z"))
            .collect();
        [v[0], v[1], v[2]]
    });
    let options = Options {
        me: Hello {
            version: m2_net::proto::VERSION,
            hero: 0,
            name: "coop_peer".into(),
        },
        relay: !flag("--local"),
        secret: None,
        relay_only: flag("--relay-only"),
    };
    let session = match value("--join") {
        Some(code) => Session::join(code.parse::<Invite>().expect("invite"), options),
        None if flag("--host") => Session::host(options),
        None => {
            eprintln!("coop_peer --host | --join <code> [--local] [--seconds s] [--at x,y,z]");
            std::process::exit(2);
        }
    };

    let started = Instant::now();
    let mut connected_at = None;
    let mut received = 0u32;
    let mut last_log = 0.0;
    let mut link = String::from("-");
    while started.elapsed().as_secs_f64() < seconds {
        for e in session.poll() {
            match e {
                Event::Invite(i) => {
                    println!("invite: {i}");
                    println!("link: {}", i.without_direct().link());
                }
                Event::Connected(h) => {
                    connected_at = Some(started.elapsed());
                    println!(
                        "connected: {} (hero {}) after {:.2} s",
                        h.name,
                        h.hero,
                        started.elapsed().as_secs_f64()
                    );
                }
                Event::Hero(h, _) => {
                    received += 1;
                    if received == 1 {
                        println!("first hero: {:?}", h.position);
                        centre.get_or_insert(h.position);
                    }
                }
                Event::Link { rtt, relayed } => {
                    link = format!(
                        "rtt {:.1} ms {}",
                        rtt.as_secs_f64() * 1e3,
                        if relayed { "relayed" } else { "direct" }
                    );
                }
                Event::Disconnected(why) => println!("disconnected: {why}"),
                Event::Failed(why) => {
                    println!("failed: {why}");
                    std::process::exit(1);
                }
            }
        }
        if connected_at.is_some()
            && let Some(centre) = centre
        {
            // Round a 6 m circle at a run, 30 snapshots a second.
            let t = session.now();
            let a = (t * 0.5) as f32;
            session.send_hero(HeroState {
                time: t,
                position: [
                    centre[0] + 6.0 * a.cos(),
                    centre[1],
                    centre[2] + 6.0 * a.sin(),
                ],
                velocity: [-3.0 * a.sin(), 0.0, 3.0 * a.cos()],
                facing: (-a.sin()).atan2(a.cos()),
                gait: Gait::Run,
                ..Default::default()
            });
        }
        let t = started.elapsed().as_secs_f64();
        if connected_at.is_some() && t - last_log >= 1.0 {
            last_log = t;
            println!("peer: t {t:.0} s, received {received}, {link}");
        }
        std::thread::sleep(Duration::from_millis(33));
    }
    session.leave();
    std::thread::sleep(Duration::from_millis(600));
}
