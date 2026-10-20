//! Online co-op in the alpha: hosting or joining ([`m2_net::Session`]),
//! sending the player's hero, and showing the other player's as a
//! [`crate::puppet::Puppet`].
//!
//! Stage one of the plan in `docs/coop-plan.md`: each player is the
//! authority for their own hero, and both games run their own island.
//! The world (people, vehicles, the contract) is not shared yet.

use std::time::{Duration, Instant};

use m2_net::proto::{Hello, HeroState, VERSION};
use m2_net::{Event, Invite, Options, Playback, Session};
use m2_render::RenderBackend;

use crate::game::Game;
use crate::hero::Hero;

/// Snapshots of the player's hero sent a second.
pub const SEND_RATE: f32 = 30.0;

/// Hosting, or joining a host.
#[derive(Clone, Debug)]
pub enum Role {
    Host,
    Join(Invite),
}

/// What `--host`/`--join` ask for.
#[derive(Clone, Debug)]
pub struct Setup {
    pub role: Role,
    pub name: String,
    /// Through the internet (the public relay); off, only this network.
    pub relay: bool,
}

/// Counts for the log and the end-of-run summary.
#[derive(Debug, Default)]
pub struct Stats {
    pub connected_after: Option<f64>,
    pub received: u32,
    pub sent: u32,
    pub rtt_ms: Vec<f64>,
    pub relayed: Option<bool>,
    /// The puppet's speed each frame it moved, m/s: how smoothly it is
    /// played back (a steady runner should give a steady speed).
    pub speeds: Vec<f32>,
}

pub struct Coop {
    session: Session,
    playback: Playback,
    hero: Hero,
    pub peer: Option<Hello>,
    pub invite: Option<Invite>,
    since_send: f32,
    started: Instant,
    last_log: f64,
    last_puppet: Option<m2_math::glam::Vec3>,
    pub stats: Stats,
}

impl std::fmt::Debug for Coop {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Coop")
            .field("peer", &self.peer)
            .field("stats", &self.stats)
            .finish_non_exhaustive()
    }
}

/// The wire's number for a hero: its place in the front end's order.
pub fn hero_index(hero: Hero) -> u8 {
    Hero::ALL.iter().position(|&h| h == hero).unwrap_or(1) as u8
}

/// The hero a wire number names (Jennifer for one this build lacks).
pub fn hero_of(index: u8) -> Hero {
    Hero::ALL.get(index as usize).copied().unwrap_or_default()
}

impl Coop {
    pub fn start(setup: &Setup, hero: Hero) -> Self {
        let options = Options {
            me: Hello {
                version: VERSION,
                hero: hero_index(hero),
                name: setup.name.clone(),
            },
            relay: setup.relay,
            secret: None,
            relay_only: false,
        };
        let session = match &setup.role {
            Role::Host => {
                eprintln!("coop: hosting ({})", reach(setup.relay));
                Session::host(options)
            }
            Role::Join(invite) => {
                eprintln!(
                    "coop: joining {} ({})",
                    invite.id.fmt_short(),
                    reach(setup.relay)
                );
                Session::join(invite.clone(), options)
            }
        };
        Self {
            session,
            playback: Playback::new(),
            hero,
            peer: None,
            invite: None,
            since_send: 0.0,
            started: Instant::now(),
            last_log: 0.0,
            last_puppet: None,
            stats: Stats::default(),
        }
    }

    /// Once a frame: what came in, the player's hero out, the puppet on.
    pub fn tick(&mut self, game: &mut Game, backend: &mut dyn RenderBackend, dt: f32) {
        for e in self.session.poll() {
            self.event(e, game, backend);
        }
        if self.peer.is_some() {
            self.since_send += dt;
            if self.since_send >= 1.0 / SEND_RATE {
                self.since_send = 0.0;
                if let Some(state) = hero_state(game, self.session.now()) {
                    self.session.send_hero(state);
                    self.stats.sent += 1;
                }
            }
        }
        if let Some(p) = &mut game.scene.puppet
            && let Some(pose) = self.playback.sample(self.session.now())
        {
            p.step(&pose, dt);
            if let Some(last) = self.last_puppet
                && dt > 0.0
            {
                self.stats.speeds.push(last.distance(p.position) / dt);
            }
            self.last_puppet = Some(p.position);
        }
        let t = self.started.elapsed().as_secs_f64();
        if self.peer.is_some() && t - self.last_log >= 1.0 {
            self.last_log = t;
            self.log_second(game);
        }
        if self.hero != game.scene.hero {
            // Switched hero in the pause menu: the friend sees it on
            // their next join (a live switch is a later stage).
            self.hero = game.scene.hero;
        }
    }

    fn event(&mut self, e: Event, game: &mut Game, backend: &mut dyn RenderBackend) {
        match e {
            Event::Invite(i) => {
                // The link carries only the id and the relay; the full
                // code (with this machine's addresses) is for a game on
                // the same network.
                eprintln!("coop: invite link {}", i.without_direct().link());
                eprintln!("coop: invite code {i}");
                self.invite = Some(i);
            }
            Event::Connected(h) => {
                let t = self.started.elapsed().as_secs_f64();
                eprintln!(
                    "coop: {} joined as {} after {t:.2} s",
                    h.name,
                    hero_of(h.hero)
                );
                self.stats.connected_after.get_or_insert(t);
                game.scene
                    .load_puppet(backend, hero_of(h.hero), h.name.clone());
                self.peer = Some(h);
                self.playback = Playback::new();
                self.last_puppet = None;
            }
            Event::Hero(s, at) => {
                self.stats.received += 1;
                self.playback.push(s, at);
            }
            Event::Link { rtt, relayed } => {
                self.stats.rtt_ms.push(rtt.as_secs_f64() * 1e3);
                self.stats.relayed = Some(relayed);
            }
            Event::Disconnected(why) => {
                eprintln!("coop: the other player is gone: {why}");
                self.peer = None;
                game.scene.puppet = None;
            }
            Event::Failed(why) => {
                eprintln!("coop: {why}");
                self.peer = None;
                game.scene.puppet = None;
            }
        }
    }

    fn log_second(&self, game: &Game) {
        let me = game
            .scene
            .player
            .as_ref()
            .map(|p| p.character.position)
            .unwrap_or_default();
        let (at, gait) = game.scene.puppet.as_ref().map_or_else(
            || ("none".to_owned(), "-".to_owned()),
            |p| {
                let at = p.position;
                let gait = p
                    .body
                    .as_ref()
                    .map_or("-".to_owned(), |(_, r)| format!("{:?}", r.gait));
                (format!("{:.1},{:.1},{:.1}", at.x, at.y, at.z), gait)
            },
        );
        eprintln!(
            "coop: t {:.0} s, sent {}, received {} ({} stale), rtt {}, {}; other hero at {at} ({gait}), {:.1} m from her",
            self.started.elapsed().as_secs_f64(),
            self.stats.sent,
            self.stats.received,
            self.playback.stale,
            self.stats
                .rtt_ms
                .last()
                .map_or("-".to_owned(), |r| format!("{r:.1} ms")),
            match self.stats.relayed {
                Some(true) => "relayed",
                Some(false) => "direct",
                None => "path unknown",
            },
            game.scene
                .puppet
                .as_ref()
                .map_or(0.0, |p| p.position.distance(me)),
        );
    }

    /// The run's numbers in one line.
    pub fn summary(&self) -> String {
        let s = &self.stats;
        let secs = self.started.elapsed().as_secs_f64() - s.connected_after.unwrap_or(0.0);
        let rtt = if s.rtt_ms.is_empty() {
            "-".to_owned()
        } else {
            format!(
                "{:.1} ms mean",
                s.rtt_ms.iter().sum::<f64>() / s.rtt_ms.len() as f64
            )
        };
        // Frames after the first second, when playback has settled.
        let speeds = &s.speeds[s.speeds.len().min(60)..];
        let (lo, hi, mean) = if speeds.is_empty() {
            (0.0, 0.0, 0.0)
        } else {
            let lo = speeds.iter().copied().fold(f32::MAX, f32::min);
            let hi = speeds.iter().copied().fold(0.0, f32::max);
            (lo, hi, speeds.iter().sum::<f32>() / speeds.len() as f32)
        };
        format!(
            "coop summary: connected after {}, {secs:.1} s together, sent {}, received {} ({:.1}/s, {} stale), rtt {rtt} ({}), the other hero's speed {lo:.2}..{hi:.2} m/s (mean {mean:.2}) over {} frames",
            s.connected_after
                .map_or("never".to_owned(), |t| format!("{t:.2} s")),
            s.sent,
            s.received,
            s.received as f64 / secs.max(1e-3),
            self.playback.stale,
            match s.relayed {
                Some(true) => "relayed",
                Some(false) => "direct",
                None => "path unknown",
            },
            speeds.len(),
        )
    }

    /// Says goodbye.
    pub fn leave(&self) {
        self.session.leave();
    }
}

fn reach(relay: bool) -> &'static str {
    if relay {
        "over the internet"
    } else {
        "this network only"
    }
}

/// The player's hero as sent: where, how fast, which way, how.
fn hero_state(game: &Game, now: f64) -> Option<HeroState> {
    let p = game.scene.player.as_ref()?;
    let gait = p
        .model
        .as_ref()
        .map_or(m2_engine::Gait::Idle, |(_, rig)| rig.gait);
    let mut flags = 0;
    if !p.character.grounded {
        flags |= HeroState::AIRBORNE;
    }
    if p.aiming {
        flags |= HeroState::AIMING;
    }
    if p.driving.is_some() || !game.on_foot {
        flags |= HeroState::DRIVING;
    }
    if p.swim.is_some() {
        flags |= HeroState::SWIMMING;
    }
    Some(HeroState {
        seq: 0, // numbered by the session
        time: now,
        position: p.character.position.to_array(),
        velocity: p.character.velocity.to_array(),
        facing: p.facing,
        gait: crate::puppet::wire_gait(gait),
        flags,
    })
}

/// `--screenshot` with `--host` or `--join`: plays on for `seconds` of
/// real time (the network's clock is the wall's), the player standing
/// or going at `--gait`, then pictures the island with the other hero in
/// it and logs the summary.
#[allow(clippy::too_many_arguments)]
pub fn offscreen(
    backend: &mut m2_render::gpu::WgpuBackend,
    game: &mut Game,
    setup: &Setup,
    seconds: f32,
    walk: Option<m2_engine::Gait>,
    fov: f32,
    aspect: f32,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let mut coop = Coop::start(setup, game.scene.hero);
    let tick = crate::TICK;
    let start = Instant::now();
    let mut next = start;
    let mut mid = false;
    while start.elapsed().as_secs_f32() < seconds {
        let intent = match walk {
            Some(g) if g != m2_engine::Gait::Idle && coop.peer.is_some() => crate::player::Intent {
                movement: m2_math::glam::Vec2::Y,
                sprint: g == m2_engine::Gait::Sprint,
                walk: g == m2_engine::Gait::Walk,
                ..Default::default()
            },
            _ => crate::player::Intent::default(),
        };
        game.step(intent, tick.as_secs_f32());
        coop.tick(game, backend, tick.as_secs_f32());
        crate::render(backend, game, fov, aspect, None)?;
        if !mid && coop.peer.is_some() && start.elapsed().as_secs_f32() > seconds * 0.5 {
            mid = true;
            crate::write_ppm(backend, &out.with_extension("mid.ppm"))?;
        }
        next += tick;
        if let Some(wait) = next.checked_duration_since(Instant::now()) {
            std::thread::sleep(wait);
        } else if Instant::now() - next > Duration::from_millis(250) {
            // Rendering is slower than real time: let the clock go.
            next = Instant::now();
        }
    }
    crate::write_ppm(backend, out)?;
    eprintln!("{}", coop.summary());
    coop.leave();
    std::thread::sleep(Duration::from_millis(600));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heroes_go_over_the_wire_by_number() {
        for h in Hero::ALL {
            assert_eq!(hero_of(hero_index(h)), h);
        }
        assert_eq!(hero_of(9), Hero::default());
    }
}
