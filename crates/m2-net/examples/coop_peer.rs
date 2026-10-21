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
//! link's round trip and whether it goes through the relay, and what of
//! the host's people came (kinds, people in and out, places).
//!
//! `--people N` sends N soldiers of its own as a host's game would,
//! walking circles round the other hero, to test a joining game's
//! mirror of the host's people.

use std::time::{Duration, Instant};

use std::collections::HashSet;

use m2_net::proto::PEOPLE_PER_DATAGRAM;
use m2_net::{
    Event, Gait, Hello, HeroState, Invite, KindInfo, Msg, Options, PeopleBatch, PersonIn,
    PersonState, Session,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let flag = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| {
        args.iter()
            .position(|a| a == f)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let people: u32 = value("--people").map_or(0, |s| s.parse().expect("--people"));
    let seconds: f64 = value("--seconds").map_or(30.0, |s| s.parse().expect("--seconds"));
    // Round this point, or round the other hero where it first is.
    let mut centre: Option<[f32; 3]> = value("--at").map(|s| {
        let v: Vec<f32> = s
            .split(',')
            .map(|v| v.parse().expect("--at x,y,z"))
            .collect();
        [v[0], v[1], v[2]]
    });
    let follow = centre.is_none();
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
    let (mut kinds, mut ins, mut outs, mut states) = (0u32, 0u32, 0u32, 0u32);
    let mut seen = HashSet::new();
    let mut announced = false;
    let mut last_people = 0.0;
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
                    }
                    // Without --at, round wherever the other hero is now.
                    if follow {
                        centre = Some(h.position);
                    }
                }
                Event::Link { rtt, relayed } => {
                    link = format!(
                        "rtt {:.1} ms {}",
                        rtt.as_secs_f64() * 1e3,
                        if relayed { "relayed" } else { "direct" }
                    );
                }
                Event::People(b, _) => {
                    states += b.people.len() as u32;
                    seen.extend(b.people.iter().map(|p| p.net_id));
                }
                Event::Message(Msg::Kind(k)) => {
                    kinds += 1;
                    println!(
                        "kind {}: {} {:08x} named {}",
                        k.id, k.name, k.model, k.named
                    );
                }
                Event::Message(Msg::PersonIn(_)) => ins += 1,
                Event::Message(Msg::PersonOut(_)) => outs += 1,
                Event::Message(m) => println!("message: {m:?}"),
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
            if people > 0 && !announced {
                announced = true;
                session.send(
                    Msg::Kind(KindInfo {
                        id: 0,
                        name: "vz_hum_soldier".into(),
                        model: 0,
                        named: false,
                    }),
                    true,
                );
                for id in 1..=people {
                    session.send(
                        Msg::PersonIn(PersonIn {
                            net_id: id,
                            kind: 0,
                            outfit: Vec::new(),
                            armed: true,
                        }),
                        true,
                    );
                }
            }
            if people > 0 && t - last_people >= 0.1 {
                last_people = t;
                // Each walks its own circle, 3 m out and 1.5 m more each,
                // the other way round at 1.4 m/s.
                let all: Vec<PersonState> = (1..=people)
                    .map(|id| {
                        let r = 3.0 + 1.5 * id as f32;
                        let a = -(t as f32) * 1.4 / r + id as f32;
                        PersonState {
                            net_id: id,
                            position: [centre[0] + r * a.cos(), centre[1], centre[2] + r * a.sin()],
                            facing: (a.sin()).atan2(-a.cos()),
                            gait: Gait::Walk,
                            flags: PersonState::ALIVE,
                        }
                    })
                    .collect();
                for chunk in all.chunks(PEOPLE_PER_DATAGRAM) {
                    session.send(
                        Msg::People(PeopleBatch {
                            time: t,
                            people: chunk.to_vec(),
                        }),
                        false,
                    );
                }
            }
        }
        let t = started.elapsed().as_secs_f64();
        if connected_at.is_some() && t - last_log >= 1.0 {
            last_log = t;
            println!(
                "peer: t {t:.0} s, received {received}, {link}; people: {kinds} kinds, {ins} in, {outs} out, {states} places of {} people",
                seen.len()
            );
        }
        std::thread::sleep(Duration::from_millis(33));
    }
    session.leave();
    std::thread::sleep(Duration::from_millis(600));
}
