//! Two sessions on this machine, without the relay: the host invites,
//! the friend joins with the invite, and each sees the other's hero.

use std::time::Duration;

use m2_net::{Event, Gait, Hello, HeroState, Options, Session};

fn options(name: &str, hero: u8) -> Options {
    Options {
        me: Hello {
            version: m2_net::proto::VERSION,
            hero,
            name: name.into(),
        },
        relay: false,
        secret: None,
        relay_only: false,
    }
}

fn next(s: &Session, what: &str) -> Event {
    s.wait(Duration::from_secs(20))
        .unwrap_or_else(|| panic!("no event while waiting for {what}"))
}

#[test]
fn two_games_see_each_others_hero() {
    let host = Session::host(options("Host", 0));
    let Event::Invite(invite) = next(&host, "the invite") else {
        panic!("expected the invite")
    };
    assert!(!invite.direct.is_empty(), "{invite}");
    // The code as a friend would paste it, through a link.
    let friend = Session::join(invite.link().parse().unwrap(), options("Friend", 2));

    let Event::Connected(h) = next(&friend, "the welcome") else {
        panic!("friend not welcomed")
    };
    assert_eq!((h.name.as_str(), h.hero), ("Host", 0));
    let Event::Connected(f) = next(&host, "the friend") else {
        panic!("host saw no friend")
    };
    assert_eq!((f.name.as_str(), f.hero), ("Friend", 2));

    // Thirty snapshots each way, a step apart along x.
    for i in 0..30 {
        let s = |x: f32| HeroState {
            time: i as f64 / 30.0,
            position: [x, 0.0, i as f32],
            gait: Gait::Run,
            ..Default::default()
        };
        host.send_hero(s(1.0));
        friend.send_hero(s(-1.0));
        std::thread::sleep(Duration::from_millis(5));
    }
    let heroes = |s: &Session, x: f32| {
        let mut got = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while got.len() < 30 && std::time::Instant::now() < deadline {
            if let Some(Event::Hero(h, _)) = s.wait(Duration::from_millis(100)) {
                assert_eq!(h.position[0], x);
                got.push(h.seq);
            }
        }
        got
    };
    let at_friend = heroes(&friend, 1.0);
    let at_host = heroes(&host, -1.0);
    // Datagrams may be lost, but not on this machine's loopback.
    assert!(at_friend.len() >= 28, "{at_friend:?}");
    assert!(at_host.len() >= 28, "{at_host:?}");

    friend.leave();
    loop {
        match next(&host, "the goodbye") {
            Event::Disconnected(why) => {
                assert!(why.contains("left"), "{why}");
                break;
            }
            Event::Hero(..) | Event::Link { .. } => {}
            other => panic!("{other:?}"),
        }
    }
}
