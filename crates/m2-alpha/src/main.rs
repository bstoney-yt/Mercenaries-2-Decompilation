//! `m2-alpha`: the native game, playing from the player's own install.
//!
//! ```text
//! m2-alpha [--install <dir>] [--spawn <x>,<z>] [--model <hash>]... [--gait <gait>]
//!          [--config <file>]
//!          [--size <w>x<h>] [--vsync on|off] [--fullscreen] [--double-sided] [--fly]
//!          [--display windowed|borderless|exclusive] [--refresh <hz>]
//!          [--sea <metres>]
//!          [--screenshot <out.ppm> [--drive | --fight]] [--write-config]
//! ```
//!
//! Settings come from `m2-alpha.toml` (in the working directory or beside
//! the executable) and the command line overrides them. `--write-config`
//! prints a settings file with every option at its current value.
//! `--screenshot` renders one frame without opening a window, for checking
//! a machine's GPU setup.
//!
//! Right now this opens a window on the island, streamed from the install
//! around the player: Jennifer on foot with a third-person camera
//! (`--spawn` picks where to start; the default is the PMC's headquarters,
//! where free roam begins after the first contract). WASD
//! runs, Shift sprints, Left Ctrl held walks, Space jumps (the PC game's
//! keys, [`input`]). E gets in and out of
//! vehicles: in free roam also those parked ahead of the spawn, with a
//! mission only those the layers and scripts place (WASD drive, Space
//! brakes, R rights an overturned one). F4 switches to a
//! free-flying camera and back (`--fly` starts flying): WASD/Space/C fly,
//! Shift speeds up. With `--model` it shows those models on a ground grid
//! instead, characters playing the game's locomotion clips in place (N
//! steps through idle, walk, run and sprint; `--gait` picks the first).
//! A click captures the mouse (F2 lets it go), F11 or Alt+Enter toggles
//! fullscreen ([`display`]), F3 toggles back-face culling, Escape releases
//! the mouse, then quits.
//!
//! `--screenshot` on the island waits for the ground, lands the player and,
//! with `--gait walk|run|sprint`, runs her forward for three seconds before
//! taking the picture, printing where she got to. With `--drive` she gets
//! in the nearest parked vehicle instead and drives it forward; with
//! `--fight` she shoots the nearest soldier, fires a rocket at a vehicle and
//! throws a grenade.

mod aim;
#[cfg(target_os = "android")]
mod android;
mod autoplay;
mod binoculars;
mod breakables;
mod cache;
mod camera;
mod cheats;
mod combat;
mod config;
mod coop;
mod destruct;
mod disguise;
mod display;
mod drops;
mod flights;
mod flow;
mod game;
mod gamehud;
mod grapple;
mod held;
mod hero;
mod hijack;
mod input;
mod jobs;
mod keytable;
mod loading;
mod mission;
mod movie;
mod moving;
mod multiply;
mod npc;
mod pad;
mod particles;
mod paths;
mod pause;
mod placed;
mod player;
mod props;
mod puppet;
mod raw;
mod reinforce;
mod ridehijack;
mod roads;
mod route;
mod save;
mod scene;
mod shell;
mod shopcheck;
mod sound;
mod spans;
mod startup;
mod strikes;
mod subtitles;
mod support;
mod touch;
mod traffic;
mod ui;

use crate::hero::Hero;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use m2_engine::physics::PLANE_SPEED;
use m2_engine::{Gait, Install, WorldSettings};
use m2_render::gpu::WgpuBackend;
use m2_render::{DrawQueue, RenderBackend};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
#[cfg(not(target_os = "android"))]
use winit::window::CursorGrabMode;
use winit::window::{Window, WindowId};

use camera::FlyCamera;
use config::Config;
use game::Game;
use input::{Action, Input};
use player::Intent;
use scene::Scene;

/// Simulation rate; rendering runs as fast as presentation allows.
const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);
/// Longest real time a single frame may advance the simulation by, so a
/// stall (dragging the window, a breakpoint) doesn't fast-forward the game.
const MAX_FRAME: Duration = Duration::from_millis(250);
/// How far off people's clip sounds (foley, footsteps) are heard, metres:
/// as far as those sounds carry.
const CUE_REACH: f32 = 35.0;

#[derive(Debug, Default)]
struct Options {
    config_path: Option<PathBuf>,
    install: Option<PathBuf>,
    models: Vec<u32>,
    /// With `--model`: a clip for each model to play instead of a gait,
    /// or to hold still this many seconds in.
    clips: Vec<(u32, Option<f32>)>,
    /// With `--model`: degrees to turn the models (90 shows their side).
    turn: f32,
    spawn: Option<(f32, f32)>,
    /// Who to play (`--hero`), over the settings'.
    hero: Option<Hero>,
    /// Online co-op (`--host`, `--join <invite>`), with `--coop-local`
    /// and `--name`.
    coop: Option<coop::Setup>,
    /// With `--screenshot` and co-op: seconds of play together before
    /// the picture (default 30).
    coop_seconds: f32,
    /// Weapons to start with, by template name (`--gun`, repeated:
    /// `Shotgun`, `Stinger`, `C4`).
    guns: Vec<String>,
    gait: Option<Gait>,
    double_sided: bool,
    fly: bool,
    /// The developer's keys in the whole game's launch too (`--dev`): the
    /// free camera, quick save and load, culling, the gait cycle.
    dev: bool,
    /// With `--alpha` (or `--mission --movies --screenshot`): the scripts'
    /// skip mode ends once their start has run (the default), rather than
    /// staying on for the session (`--skip-mode-on`). Left on, the flow
    /// skips the fade before the villa's closing movie and the move into
    /// the PMC HQ, and Fiona's Talk never comes.
    skip_mode_off: bool,
    /// With `--mission`: the story skipped to this contract the game's
    /// developer way, and whether to its briefing (`--skip-to <id>[,briefing]`).
    skip_to: Option<(String, bool)>,
    /// With `--skip-to` a job: she is put this far from its nearest
    /// target once it is tracked (`--job-near <m>`, a test shortcut).
    job_near: Option<f32>,
    drive: bool,
    /// With `--screenshot` on foot: jump just before the picture.
    jump: bool,
    /// With `--screenshot` on foot: press jump ten times a second all the
    /// way (a player spamming it), each take-off in the log.
    jump_spam: bool,
    /// With `--screenshot` on foot: throw a grenade just before it.
    throw: bool,
    /// With `--screenshot` on foot: bash just before it.
    bash: bool,
    /// With `--screenshot` on foot: change the rifle's clip just before it.
    reload: bool,
    /// With `--bash`: the clip (by hash) to bash with, rather than the one
    /// chosen.
    bash_clip: Option<u32>,
    /// With `--jump`, `--throw` or `--bash`: seconds before the picture she
    /// starts.
    lead: Option<f32>,
    fight: bool,
    /// With `--screenshot`: rocket the nearest fuel tank or barrel.
    blast: bool,
    /// With `--screenshot`: rocket the nearest building until it comes
    /// down, at most this many rockets.
    demolish: Option<u32>,
    /// With `--drive`: park the vehicle facing the nearest thing of this
    /// kind it breaks or shoves (fence, plant, lamp, box) and drive
    /// straight through it.
    plough: Option<String>,
    /// With `--drive`: wreck it at speed at the end and watch it stop.
    wreck: bool,
    /// With `--drive`: stop and put this many rifle rounds into its parts.
    shoot_up: Option<u32>,
    /// With `--drive`: fire from the next seat with a weapon.
    gunner: bool,
    /// With `--drive`: a vehicle whose name holds this.
    vehicle: Option<String>,
    /// With `--drive`: swing the camera this far (radians) before the picture.
    look: f32,
    /// With `--screenshot`: how many of the nearest spawns to list.
    list_spawns: usize,
    /// With `--fight`: seconds to wait after it before the picture; with
    /// `--blast`, after the rocket's blast.
    fight_wait: f32,
    /// With `--screenshot`: seconds of play to watch the traffic for.
    traffic: Option<f32>,
    /// With `--screenshot` on foot: face this way (radians from +z towards
    /// +x) before the picture.
    face: Option<f32>,
    /// With `--screenshot` on foot: tilt the camera this far (radians)
    /// before the picture, to frame the sky.
    pitch: Option<f32>,
    /// With `--screenshot` on foot: hold the weapon up to aim at the end.
    aim: bool,
    /// With `--screenshot` on foot: the weapon she has up.
    weapon: Option<combat::Weapon>,
    /// With `--screenshot` on foot: switch to the other weapon (`--lead`
    /// seconds before the picture), or pick a weapon up then.
    switch: bool,
    pickup: bool,
    /// Weapons held as worked out from the hands (`--grip hands`), not
    /// on the hand's attach node as posed.
    hands_grip: bool,
    /// With `--screenshot` on foot: take the picture close up, from her
    /// right and a little ahead, at chest height, to see her hands.
    closeup: bool,
    /// With `--screenshot` on foot: take the picture close up from behind
    /// her, to see what hangs on her back.
    behind: bool,
    /// With `--screenshot` on foot: give her this weapon in the
    /// launcher's place without taking it up (`--slung gl`: the crate's
    /// grenade launcher on her back while the carbine is up).
    slung: Option<combat::Weapon>,
    /// With `--screenshot`: cast rays around where she stands and list
    /// what they hit, to find unseen obstacles.
    probe: bool,
    /// `--riders`: seat the vehicles' riders before the picture.
    riders: bool,
    /// `--take` (with `--riders`): she takes the nearest seated driver's
    /// vehicle before the picture.
    take: bool,
    /// With `--aim` (or `--drive --look`): fire just before the picture.
    fire: bool,
    /// With `--screenshot` on foot: hits she takes just before the
    /// picture, from these directions (degrees clockwise from where the
    /// camera looks), to see the HUD's hit arcs.
    hit_from: Vec<f32>,
    /// With `--screenshot`: seconds the game's HUD runs before the
    /// picture (2, or 0.5 with `--hit-from`).
    hud_seconds: Option<f64>,
    /// With `--screenshot`: the picture is of the loading screen this many
    /// seconds after it came up (as at the start, the island loading).
    loading_seconds: Option<f64>,
    /// With `--screenshot`: the loading screen comes up over the world
    /// (its wipe plays) and the picture is this many seconds after; with
    /// a second number the screen is let go then (its wipe back plays)
    /// and the picture is that many seconds later.
    loading_wipe: Option<(f64, Option<f64>)>,
    /// With `--screenshot` on foot: call a Satellite Guided Bomb on the
    /// spot ahead of her, the picture with the support menu open (menu),
    /// through the satellite's view, with the timing wheel up, or after
    /// the bomb lands.
    satellite: Option<SatelliteShot>,
    /// With `--screenshot`: the picture with her PDA open.
    pda: bool,
    /// With `--screenshot` on foot: the picture through her binoculars at
    /// this zoom, their screen up ([`binoculars`]).
    binoculars: Option<u8>,
    /// With `--screenshot`: the picture with the game's pause menu open,
    /// `--pda-key`'s keys typed into it one by one.
    pause: bool,
    /// With `--pda`: keys pressed (Flash key codes, 36 the map's current
    /// contract, 87 W to move the map) half way through the HUD's run and
    /// held to its end.
    pda_keys: Vec<i32>,
    /// Clicks among them (`--pda-click x,y`, screen pixels): the key slot
    /// each takes, and where.
    pda_clicks: Vec<(usize, [f32; 2])>,
    /// With `--mission --screenshot`: take the picture from this far from
    /// the nearest marker, looking at it.
    mission_look: Option<f32>,
    /// With `--screenshot`: take the picture from this far from the
    /// pickup lying nearest her, looking down at it.
    look_pickup: Option<f32>,
    mission: bool,
    mission_drivers: Vec<(String, String)>,
    /// Every hijackable vehicle is crewed by this human template, to test
    /// hijacking ([`hijack`]) before the island's vehicles have crews.
    hijack_crew: Option<String>,
    /// `--support`: support items to stock and call in once the scripts
    /// run, as if bought, for testing.
    support: Vec<String>,
    /// `--shop-check all|<key,...>`: with `--mission`, every support item
    /// (or those) bought and called in, one after another
    /// ([`shopcheck`]), for testing.
    shop_check: Option<Vec<String>>,
    /// `--cheat-check`: with `--mission`, every cheat of the menu tried
    /// once she is ashore ([`cheats::Check`]), for testing.
    cheat_check: bool,
    /// `--bombs`: with `--mission --autoplay`, whenever she drives a
    /// vehicle with a bomb bay, she drops a bomb every two seconds, for
    /// testing.
    bombs: bool,
    /// `--cheat-menu <page>`: with `--screenshot`, the cheat menu up at
    /// that page, over plain black.
    cheat_menu: Option<cheats::Tab>,
    /// With `--screenshot --drive`: hijack the vehicle instead of getting
    /// in, answering every prompt at once.
    hijack: bool,
    /// `--grapple`: she may grapple without the scripts' leave (testing;
    /// with `--screenshot --drive --hijack`, she grapples to the nearest
    /// crewed helicopter from 40 m and hijacks it).
    grapple: bool,
    /// With `--mission`: try every placed pickup, crate and emplaced gun
    /// as the layers put them out ([`placed::check`]), for testing.
    placed_check: bool,
    /// With `--mission`: once the scripts offer a context action whose
    /// prompt reads this (`exit`, `talk`), put her at its object and take
    /// it with the use key, logging where she is after
    /// ([`context_check`]), for testing.
    context: Option<String>,
    /// With `--mission`: the keys pressed in a dialog box the scripts put
    /// up (`up`, `down`, `accept`, `back`, comma-separated), for testing.
    dialog_keys: Option<String>,
    /// With `--screenshot` on foot and `--gait`: she goes on this long
    /// (seconds, default 3), logging where she is each second.
    walk_seconds: Option<f32>,
    /// In play (the window, not `--screenshot`): open the pause menu as
    /// Escape would, this many seconds after play begins. For testing what
    /// pausing asks of the sound (the pause music).
    pause_at: Option<f32>,
    /// With `--screenshot` on foot and `--gait`: put the nearest soldier
    /// this many metres ahead of her at the start, at the water's height
    /// there, and log each second whether he swims (`npc-swim:` lines).
    /// For testing.
    npc_swim: Option<f32>,
    /// With `--mission --after-contract`: this many seconds after the
    /// contract, take the pause menu's Medevac (if the scripts allow it).
    medevac: Option<f32>,
    /// With `--mission --after-contract`: visit a faction's soldiers (`All`,
    /// `Oil`...) and hurt one, logging their sides (faction-check lines).
    visit_faction: Option<String>,
    /// `--bash-target <s>`: that long after the contract, bash the person
    /// the scripts' first marker is over.
    bash_target: Option<f32>,
    /// `--bash-target <s>k`: knock them down instead (as a bash does a
    /// boss it doesn't hurt), to try the capture on anyone.
    knock_target: bool,
    /// `--bash-target <s>d`: kill them instead, to try the photo verify.
    kill_target: bool,
    /// `--call-extraction <s>`: that long after the contract, call in the
    /// extraction helicopter and go to it once it is down.
    call_extraction: Option<f32>,
    /// With `--mission --autoplay --after-contract`: the Enter pressed
    /// every four seconds after the contract jumps too, as the pad's A
    /// does; where she is while the scripts hold her controls is logged
    /// (`held:` lines). For testing.
    jump_with_enter: bool,
    /// With `--mission`: hit every placed prop as `--props-check` says
    /// (`vehicle`, `gun` or `blast`), once she stands in `--props-area`
    /// (x, z corners) if given, and stop ([`breakables::check`]).
    props_check: Option<breakables::HitKind>,
    props_area: Option<[f32; 4]>,
    /// With `--screenshot --drive`: load the seats' clips first, so she
    /// is seen sitting in the vehicle as she drives it.
    seated: bool,
    mission_seconds: f32,
    /// With `--mission --autoplay`: she plays on through this many
    /// contracts (the first, then the story's next ones) before she stops
    /// (`--contracts`, default 1).
    contracts: u32,
    /// A save to start from (`--load`): her place, the world's wrecks
    /// and, with the scripts, their own data.
    load: Option<save::SaveGame>,
    /// Seconds into play to load the player's own save, as the pause menu does
    /// (`--load-at`).
    load_at: Option<f32>,
    /// With `--screenshot`, where to write a save once the run ends
    /// (`--save-to`).
    save_to: Option<PathBuf>,
    /// The whole game as it launches: the boot movies, the front end, the
    /// first contract with its movies, then on (also with no options).
    alpha: bool,
    /// `--alpha` without the boot movies and front end.
    no_boot: bool,
    /// Starts at the front end, without the boot movies (`--to-front-end`:
    /// what the pause menu's Quit starts the alpha again with).
    to_front_end: bool,
    /// With `--mission --screenshot`: the scripts' movies wait for the
    /// game, which opens each, decodes a few frames and pictures one.
    movies: bool,
    /// With `--screenshot`: the front end alone (the shell's menus),
    /// pictured as `--shell-key`s are pressed.
    front_end: bool,
    /// Flash key codes pressed in the front end, two seconds apart
    /// ([`CLICK`] for a click, at its place in `shell_clicks`).
    shell_keys: Vec<i32>,
    /// Clicks in the front end (`--shell-click x,y`, screen pixels): the
    /// key slot each takes, and where.
    shell_clicks: Vec<(usize, [f32; 2])>,
    /// With `--mission --autoplay --screenshot`: seconds to play on after
    /// the contract is done.
    after_contract: f32,
    /// Our stand-in HUD instead of the game's own.
    old_hud: bool,
    /// Fire and smoke as the old spheres, not the game's own effects (to
    /// compare).
    old_fx: bool,
    /// No fire or smoke drawn at all (to measure what they cover).
    no_fx: bool,
    /// With `--mission --screenshot`: her health drops to this eight
    /// seconds in, to see the scripts' healing and survival rescue.
    hurt: Option<f32>,
    /// With `--mission`: driving, the horn is held half a second every
    /// this many seconds (to hear it, and what the gates, the disguise
    /// and friendly soldiers make of it).
    honk_every: Option<f32>,
    /// With `--mission --screenshot`: she plays the mission herself, by
    /// its markers, until it is done or the seconds are up.
    autoplay: bool,
    /// Switch the smarter enemy soldiers on (the settings'
    /// `[enhanced.smarter_ai] enabled`).
    smarter_ai: bool,
    /// With `--screenshot`: bring down the mission's gates as the bomb
    /// does, logging where their pieces go ([`gates_for_screenshot`]).
    gates: bool,
    sea: Option<f32>,
    size: Option<(u32, u32)>,
    vsync: Option<bool>,
    fullscreen: bool,
    /// `--display`: windowed, borderless or exclusive.
    display: Option<display::DisplayMode>,
    /// `--refresh`: exclusive fullscreen's rate.
    refresh: Option<u32>,
    screenshot: Option<PathBuf>,
    write_config: bool,
}

/// How far a `--satellite` call goes with the bomb.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SatelliteShot {
    /// The support menu open, the bomb in it.
    Menu,
    /// Looking through the satellite, over the spot.
    View,
    /// The timing wheel, after the first press.
    Wheel,
    /// The bomb landed.
    Bomb,
}

/// A `--satellite` call's spot lies this far ahead of her, metres.
const SATELLITE_AHEAD: f32 = 45.0;

/// The install its installer recorded in the registry (`Install Dir`,
/// seen on one real install; whether every edition's installer writes
/// it is not checked), for a game put somewhere other than Program Files.
fn registry_install() -> Option<Install> {
    const KEYS: [&str; 2] = [
        r"SOFTWARE\WOW6432Node\EA Games\Mercenaries 2 World in Flames",
        r"SOFTWARE\EA Games\Mercenaries 2 World in Flames",
    ];
    KEYS.iter()
        .filter_map(|k| m2_sys::registry::local_machine_string(k, "Install Dir"))
        .find_map(|dir| Install::find(std::path::Path::new(&dir)).ok())
}

fn parse_args(given: &[String]) -> anyhow::Result<Options> {
    let mut o = Options {
        skip_mode_off: true,
        coop_seconds: 30.0,
        ..Options::default()
    };
    let (mut coop_role, mut coop_local, mut coop_name) = (None, false, None);
    let mut args = given.iter().cloned();
    while let Some(arg) = args.next() {
        let mut value = || args.next().with_context(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--config" => o.config_path = Some(value()?.into()),
            "--install" => o.install = Some(value()?.into()),
            "--model" => {
                let v = value()?;
                o.models.push(
                    u32::from_str_radix(v.trim_start_matches("0x"), 16)
                        .with_context(|| format!("--model {v}: expected a hex hash"))?,
                );
            }
            "--clip" => {
                let v = value()?;
                let (hash, at) = match v.split_once('@') {
                    Some((h, t)) => (
                        h,
                        Some(
                            t.parse()
                                .with_context(|| format!("--clip {v}: expected seconds after @"))?,
                        ),
                    ),
                    None => (v.as_str(), None),
                };
                o.clips.push((
                    u32::from_str_radix(hash.trim_start_matches("0x"), 16)
                        .with_context(|| format!("--clip {v}: expected a hex hash"))?,
                    at,
                ));
            }
            "--turn" => o.turn = value()?.parse().context("--turn expects degrees")?,
            "--gun" => o.guns.push(value()?),
            "--host" => coop_role = Some(coop::Role::Host),
            "--join" => {
                let v = value()?;
                let invite = v
                    .parse::<m2_net::Invite>()
                    .with_context(|| format!("--join expects an invite code or link, not {v}"))?;
                coop_role = Some(coop::Role::Join(invite));
            }
            "--coop-local" => coop_local = true,
            "--name" => coop_name = Some(value()?),
            "--coop-seconds" => {
                o.coop_seconds = value()?.parse().context("--coop-seconds expects seconds")?
            }
            "--hero" => {
                let v = value()?;
                o.hero = Some(Hero::parse(&v).with_context(|| {
                    format!("--hero expects mattias, jennifer or chris, not {v}")
                })?);
            }
            "--spawn" => {
                let v = value()?;
                let (x, z) = v.split_once(',').context("--spawn expects X,Z")?;
                o.spawn = Some((x.trim().parse()?, z.trim().parse()?));
            }
            "--gait" => {
                o.gait = Some(match value()?.as_str() {
                    "idle" => Gait::Idle,
                    "walk" => Gait::Walk,
                    "run" => Gait::Run,
                    "sprint" => Gait::Sprint,
                    v => bail!("--gait {v}: expected idle, walk, run or sprint"),
                });
            }
            "--size" => {
                let v = value()?;
                let (w, h) = v.split_once('x').context("--size expects WxH")?;
                o.size = Some((w.parse()?, h.parse()?));
            }
            "--vsync" => {
                o.vsync = Some(match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => bail!("--vsync {v}: expected on or off"),
                })
            }
            "--fullscreen" => o.fullscreen = true,
            "--display" => {
                o.display = Some(
                    value()?
                        .parse()
                        .map_err(|e: String| anyhow::anyhow!("--display {e}"))?,
                )
            }
            "--refresh" => o.refresh = Some(value()?.parse().context("--refresh expects hertz")?),
            "--double-sided" => o.double_sided = true,
            "--fly" => o.fly = true,
            "--dev" => o.dev = true,
            // The default since round 25; kept so old launch lines work.
            "--skip-mode-off" => o.skip_mode_off = true,
            "--skip-mode-on" => o.skip_mode_off = false,
            "--skip-to" => {
                let v = value()?;
                o.skip_to = Some(match v.split_once(',') {
                    Some((id, "briefing")) => (id.to_owned(), true),
                    Some(_) => bail!("--skip-to expects <contract>[,briefing], not {v}"),
                    None => (v, false),
                });
            }
            "--job-near" => o.job_near = Some(value()?.parse()?),
            "--drive" => o.drive = true,
            "--gunner" => o.gunner = true,
            "--jump" => o.jump = true,
            "--jump-spam" => o.jump_spam = true,
            "--throw" => o.throw = true,
            "--bash" => o.bash = true,
            "--reload" => o.reload = true,
            "--bash-clip" => {
                let v = value()?;
                o.bash_clip = Some(
                    u32::from_str_radix(v.trim_start_matches("0x"), 16)
                        .context("--bash-clip expects a clip's hash (8 hex digits)")?,
                );
            }
            "--lead" => o.lead = Some(value()?.parse().context("--lead expects seconds")?),
            "--fight" => o.fight = true,
            "--blast" => o.blast = true,
            "--demolish" => {
                o.demolish = Some(
                    value()?
                        .parse()
                        .context("--demolish expects a rocket count")?,
                )
            }
            "--plough" => o.plough = Some(value()?.to_ascii_lowercase()),
            "--wreck" => o.wreck = true,
            "--shoot-up" => {
                o.shoot_up = Some(
                    value()?
                        .parse()
                        .context("--shoot-up expects a round count")?,
                )
            }
            "--vehicle" => o.vehicle = Some(value()?.to_ascii_lowercase()),
            "--look" => {
                o.look = value()?
                    .parse::<f32>()
                    .context("--look expects degrees")?
                    .to_radians()
            }
            "--mission" => o.mission = true,
            "--alpha" => o.alpha = true,
            "--no-boot" => o.no_boot = true,
            "--to-front-end" => o.to_front_end = true,
            "--movies" => o.movies = true,
            "--front-end" => o.front_end = true,
            "--shell-key" => o.shell_keys.push(
                value()?
                    .parse()
                    .context("--shell-key expects a Flash key code")?,
            ),
            "--shell-click" => {
                let v = value()?;
                let (x, y) = v
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
                    .context("--shell-click expects x,y")?;
                o.shell_clicks.push((o.shell_keys.len(), [x, y]));
                o.shell_keys.push(CLICK);
            }
            "--after-contract" => o.after_contract = value()?.parse()?,
            "--old-hud" => o.old_hud = true,
            "--old-fx" => o.old_fx = true,
            "--no-fx" => o.no_fx = true,
            "--hijack-crew" => o.hijack_crew = Some(value()?),
            "--hijack" => o.hijack = true,
            "--grapple" => o.grapple = true,
            "--placed-check" => o.placed_check = true,
            "--context" => o.context = Some(value()?),
            "--dialog-keys" => o.dialog_keys = Some(value()?),
            "--medevac" => o.medevac = Some(value()?.parse()?),
            "--visit-faction" => o.visit_faction = Some(value()?),
            "--call-extraction" => o.call_extraction = Some(value()?.parse()?),
            "--bash-target" => {
                let v = value()?;
                o.knock_target = v.ends_with('k');
                o.kill_target = v.ends_with('d');
                o.bash_target = Some(v.trim_end_matches(['k', 'd']).parse()?);
            }
            "--jump-with-enter" => o.jump_with_enter = true,
            "--props-check" => {
                let v = value()?;
                o.props_check = Some(breakables::HitKind::parse(&v).with_context(|| {
                    format!("--props-check expects vehicle, gun or blast, not {v}")
                })?);
            }
            "--props-area" => {
                let v: Vec<f32> = value()?
                    .split(',')
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .context("--props-area expects x0,z0,x1,z1")?;
                o.props_area = Some(
                    v.try_into()
                        .map_err(|_| anyhow::anyhow!("--props-area expects x0,z0,x1,z1"))?,
                );
            }
            "--seated" => o.seated = true,
            "--load" => o.load = Some(save::SaveGame::read(&PathBuf::from(value()?))?),
            "--save-to" => o.save_to = Some(value()?.into()),
            "--load-at" => o.load_at = Some(value()?.parse().context("--load-at expects seconds")?),
            "--mission-driver" => {
                let path = value()?;
                let code =
                    std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
                o.mission_drivers.push((path, code));
            }
            "--support" => o.support.push(value()?),
            "--cheat-check" => o.cheat_check = true,
            "--bombs" => o.bombs = true,
            "--cheat-menu" => {
                let v = value()?;
                o.cheat_menu = Some(cheats::Tab::named(&v).with_context(|| {
                    format!("--cheat-menu expects weapons, vehicles, support or player, not {v}")
                })?);
            }
            "--shop-check" => {
                let v = value()?;
                o.shop_check = Some(if v == "all" {
                    Vec::new()
                } else {
                    v.split(',').map(str::to_owned).collect()
                });
            }
            "--spawns" => o.list_spawns = value()?.parse().context("--spawns expects a count")?,
            "--face" => {
                o.face = Some(
                    value()?
                        .parse::<f32>()
                        .context("--face expects degrees")?
                        .to_radians(),
                )
            }
            "--pitch" => {
                o.pitch = Some(
                    value()?
                        .parse::<f32>()
                        .context("--pitch expects degrees")?
                        .to_radians(),
                )
            }
            "--aim" => o.aim = true,
            "--hit-from" => {
                for d in value()?.split(',') {
                    o.hit_from
                        .push(d.parse().context("--hit-from expects degrees")?);
                }
            }
            "--hud-seconds" => {
                o.hud_seconds = Some(value()?.parse().context("--hud-seconds expects seconds")?)
            }
            "--loading-screen" => {
                o.loading_seconds = Some(
                    value()?
                        .parse()
                        .context("--loading-screen expects seconds")?,
                )
            }
            "--loading-wipe" => {
                let v = value()?;
                let mut parts = v.split(',').map(str::parse::<f64>);
                let up = parts
                    .next()
                    .and_then(Result::ok)
                    .context("--loading-wipe expects seconds[,seconds]")?;
                let out = parts
                    .next()
                    .transpose()
                    .context("--loading-wipe expects seconds[,seconds]")?;
                o.loading_wipe = Some((up, out));
            }
            "--weapon" => {
                o.weapon = Some(match value()?.as_str() {
                    "rifle" => combat::Weapon::Rifle,
                    "rpg" => combat::Weapon::Rpg,
                    "gl" => combat::Weapon::GrenadeLauncher,
                    v => bail!("--weapon {v}: expected rifle, rpg or gl"),
                })
            }
            "--switch" => o.switch = true,
            "--pickup" => o.pickup = true,
            "--closeup" => o.closeup = true,
            "--pda" => o.pda = true,
            "--pause" => o.pause = true,
            "--binoculars" => {
                o.binoculars = Some(
                    value()?
                        .parse()
                        .context("--binoculars expects a zoom level")?,
                )
            }
            "--pda-key" => o.pda_keys.push(
                value()?
                    .parse()
                    .context("--pda-key expects a Flash key code")?,
            ),
            "--pda-click" => {
                let v = value()?;
                let (x, y) = v
                    .split_once(',')
                    .and_then(|(x, y)| Some((x.trim().parse().ok()?, y.trim().parse().ok()?)))
                    .context("--pda-click expects x,y")?;
                o.pda_clicks.push((o.pda_keys.len(), [x, y]));
                o.pda_keys.push(CLICK);
            }
            "--behind" => o.behind = true,
            "--slung" => {
                o.slung = Some(match value()?.as_str() {
                    "rpg" => combat::Weapon::Rpg,
                    "gl" => combat::Weapon::GrenadeLauncher,
                    v => bail!("--slung {v}: expected rpg or gl"),
                })
            }
            "--satellite" => {
                o.satellite = Some(match value()?.as_str() {
                    "menu" => SatelliteShot::Menu,
                    "view" => SatelliteShot::View,
                    "wheel" => SatelliteShot::Wheel,
                    "bomb" => SatelliteShot::Bomb,
                    v => bail!("--satellite {v}: expected menu, view, wheel or bomb"),
                })
            }
            "--grip" => {
                o.hands_grip = match value()?.as_str() {
                    "attach" => false,
                    "hands" => true,
                    v => bail!("--grip {v}: expected attach or hands"),
                }
            }
            "--probe" => o.probe = true,
            "--riders" => o.riders = true,
            "--take" => o.take = true,
            "--fire" => o.fire = true,
            "--look-pickup" => {
                o.look_pickup = Some(value()?.parse().context("--look-pickup expects metres")?)
            }
            "--mission-look" => {
                o.mission_look = Some(value()?.parse().context("--mission-look expects metres")?)
            }
            "--traffic" => o.traffic = Some(value()?.parse().context("--traffic expects seconds")?),
            "--fight-wait" => {
                o.fight_wait = value()?.parse().context("--fight-wait expects seconds")?
            }
            "--autoplay" => o.autoplay = true,
            "--smarter-ai" => o.smarter_ai = true,
            "--hurt" => o.hurt = Some(value()?.parse().context("--hurt expects health")?),
            "--honk-every" => {
                o.honk_every = Some(value()?.parse().context("--honk-every expects seconds")?)
            }
            "--gates" => o.gates = true,
            "--walk-seconds" => o.walk_seconds = Some(value()?.parse()?),
            "--pause-at" => o.pause_at = Some(value()?.parse()?),
            "--contracts" => {
                o.contracts = value()?.parse().context("--contracts expects a count")?
            }
            "--npc-swim" => {
                o.npc_swim = Some(value()?.parse().context("--npc-swim expects metres")?)
            }
            "--mission-seconds" => {
                o.mission_seconds = value()?
                    .parse()
                    .context("--mission-seconds expects seconds")?
            }
            "--sea" => o.sea = Some(value()?.parse().context("--sea expects metres")?),
            "--screenshot" => o.screenshot = Some(value()?.into()),
            "--write-config" => o.write_config = true,
            "-h" | "--help" => {
                println!("{}", include_str!("usage.txt"));
                std::process::exit(0);
            }
            _ => bail!("unknown argument {arg} (try --help)"),
        }
    }
    // Launched plainly (only where the game is, the window and settings
    // named): the whole game, as `--alpha`.
    const PLAIN: [&str; 17] = [
        "--install",
        "--config",
        "--size",
        "--fullscreen",
        "--display",
        "--refresh",
        "--vsync",
        "--no-boot",
        "--to-front-end",
        "--smarter-ai",
        "--dev",
        "--skip-mode-off",
        "--skip-mode-on",
        // Co-op plays the whole game too.
        "--host",
        "--join",
        "--coop-local",
        "--name",
    ];
    if given
        .iter()
        .all(|a| !a.starts_with("--") || PLAIN.contains(&a.as_str()))
    {
        o.alpha = true;
    }
    if o.alpha {
        o.mission = true;
    }
    if coop_role.is_none() && o.alpha {
        // The join page copies `m2coop:<code>` before the download: the
        // whole game, launched plain, joins it.
        coop_role = m2_sys::clipboard::text()
            .filter(|t| t.trim_start().starts_with("m2coop:"))
            .and_then(|t| t.parse::<m2_net::Invite>().ok())
            .map(|invite| {
                eprintln!("coop: joining the invite on the clipboard");
                coop::Role::Join(invite)
            });
    }
    o.coop = coop_role.map(|role| coop::Setup {
        role,
        name: coop_name.unwrap_or_else(|| "Player".into()),
        relay: !coop_local,
        copy_link: o.screenshot.is_none(),
    });
    Ok(o)
}

/// Panics are also written to `m2-alpha-crash.log` (beside the settings
/// file or the exe), with a backtrace, since a windowed game has no console
/// to read.
fn keep_crash_reports() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        let path = Config::file_beside("m2-alpha-crash.log");
        let report = format!(
            "m2-alpha {} crashed: {info}\n\n{}\n",
            env!("CARGO_PKG_VERSION"),
            std::backtrace::Backtrace::force_capture()
        );
        if std::fs::write(&path, report).is_ok() {
            eprintln!("crash report written to {}", path.display());
        }
    }));
}

/// When `--hurt` takes her health: after the start's fade, which keeps her
/// from harm.
const HURT_AT: f32 = 8.0;

/// With `--mission --hijack`, how long an action marker's vehicle is left
/// before she hijacks it, seconds: the villa tank comes through the hall
/// wall first (in the PC footage it bursts in 4 s before "Hijack tank" is
/// added; inferred, not measured from the game).
const HIJACK_WAIT: f32 = 8.0;
/// With --autoplay --hijack: her use key starts a hijack within this of
/// the marked vehicle's middle, metres (invented: a tank's hull is about
/// 7 m long).
const HIJACK_REACH: f32 = 6.0;

/// With `--mission --placed-check`, how often what the layers put out is
/// tried, seconds.
const PLACED_CHECK_EVERY: f32 = 5.0;

/// With `--mission --props-check`, how long she is ashore where the props
/// are hit before they are, seconds: for the contract's layers to be in.
const PROPS_CHECK_AFTER: f32 = 20.0;

// The Android build's entry point is `android::android_main`.
#[cfg_attr(target_os = "android", allow(dead_code))]
fn main() -> anyhow::Result<()> {
    keep_crash_reports();
    let args: Vec<String> = std::env::args().skip(1).collect();
    run(&args, EventLoop::new)
}

/// The game, given its arguments and how to make its event loop (the
/// Android build hands over the activity's).
fn run(
    args: &[String],
    event_loop: impl FnOnce() -> Result<EventLoop<()>, winit::error::EventLoopError>,
) -> anyhow::Result<()> {
    let mut options = parse_args(args)?;
    combat::use_attach_frame(!options.hands_grip);
    let config_path = options
        .config_path
        .clone()
        .unwrap_or_else(Config::default_path);
    let mut config = Config::load(&config_path)?;
    if let Some(hero) = options.hero {
        config.hero = hero;
    }
    if let Some(install) = &options.install {
        config.install = Some(install.clone());
    }
    if let Some((w, h)) = options.size {
        config.window.width = w;
        config.window.height = h;
    }
    if let Some(v) = options.vsync {
        config.window.vsync = v;
    }
    if options.fullscreen && config.window.mode == display::DisplayMode::Windowed {
        config.window.mode = display::DisplayMode::Borderless;
    }
    if let Some(mode) = options.display {
        config.window.mode = mode;
    }
    if let Some(hz) = options.refresh {
        config.window.refresh = hz;
    }
    if let Some(sea) = options.sea {
        config.world.sea_level = sea;
    }
    config.enhanced.smarter_ai.enabled |= options.smarter_ai;
    if options.write_config {
        print!("{}", config.to_toml());
        return Ok(());
    }
    let install = match &config.install {
        Some(dir) => Some(Install::find(dir)?),
        None => match Install::detect().or_else(registry_install) {
            Some(found) => {
                eprintln!("found the game in {}", found.root.display());
                Some(found)
            }
            None => {
                eprintln!(
                    "Mercenaries 2 not found: set `install` in {} or pass --install <dir>. Showing the ground only.",
                    config_path.display()
                );
                None
            }
        },
    };
    // No models named: the island (or, without an install, Jennifer's
    // grid, which shows only the ground).
    let gait = options.gait.unwrap_or(Gait::Idle);
    let mode = if !options.models.is_empty() {
        Mode::Models(Showcase {
            models: options.models.clone(),
            gait,
            clips: options.clips.clone(),
            turn: options.turn,
        })
    } else if install.is_some() {
        // Free roam starts at the PMC's headquarters, a mission at the
        // first contract's spawn point (both found in the layers).
        Mode::Island(options.spawn)
    } else {
        Mode::Models(Showcase {
            models: vec![scene::DEFAULT_MODEL],
            gait,
            clips: Vec::new(),
            turn: 0.0,
        })
    };

    if let (Some(out), true, Some(install)) = (&options.screenshot, options.front_end, &install) {
        // Continue or Load Game picked: play starts from that save, as
        // `--load` starts it.
        let input = (&options.shell_keys[..], &options.shell_clicks[..]);
        let Some(path) = front_end_offscreen(&config, install, input, out)? else {
            return Ok(());
        };
        let s = save::SaveGame::read(&path)?;
        config.hero = s.hero.unwrap_or(config.hero);
        eprintln!(
            "front end: play starts from {} as {}",
            path.display(),
            config.hero
        );
        options.load = Some(s);
        let out = out.with_file_name(format!(
            "{}-play.ppm",
            out.file_stem()
                .map_or("shot".into(), |s| s.to_string_lossy())
        ));
        return screenshot(&config, Some(install), &mode, &options, &out);
    }
    if let Some(out) = &options.screenshot {
        return screenshot(&config, install.as_ref(), &mode, &options, out);
    }

    let event_loop = event_loop()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        config,
        install,
        mode,
        double_sided: options.double_sided,
        fly: options.fly,
        // The whole game plays as it launches, without the developer's
        // keys, unless asked for; the test launches keep them.
        dev: options.dev || options.fly || !options.alpha,
        grapple: options.grapple,
        mission: options.mission,
        mission_drivers: options.mission_drivers.clone(),
        coop: options.coop.clone(),
        old_hud: options.old_hud,
        hijack_crew: options.hijack_crew.clone(),
        support: options.support.clone(),
        alpha: options.alpha,
        no_boot: options.no_boot,
        to_front_end: options.to_front_end,
        skip_mode_off: options.skip_mode_off,
        skip_to: options.skip_to.clone(),
        guns: options.guns.clone(),
        load: options.load.clone(),
        load_at: options.load_at,
        pause_at: options.pause_at,
        successor: None,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    let successor = app.successor.take();
    let error = app.error.take();
    // The window, the world and the GPU go before waiting.
    drop(app);
    if let Some(child) = successor {
        wait_for_successor(child);
    }
    match error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Waits for the alpha started again in this one's place. Started from a
/// console (the launcher's), it shares that console, which closes, taking
/// it with it, once the process the console was opened for (this one)
/// ends: so this one stays, windowless, until it is done.
fn wait_for_successor(mut child: std::process::Child) {
    match child.wait() {
        Ok(status) => eprintln!("quit: the alpha started again ended ({status})"),
        Err(e) => eprintln!("quit: could not wait for the alpha started again: {e}"),
    }
}

/// What to show.
#[derive(Clone, Debug)]
enum Mode {
    /// The streamed island, starting above (x, z), or at the PMC's
    /// headquarters.
    Island(Option<(f32, f32)>),
    /// These models on a ground grid, characters in this gait.
    Models(Showcase),
}

/// Models shown side by side (`--model`): what characters play.
#[derive(Clone, Debug)]
pub struct Showcase {
    pub models: Vec<u32>,
    pub gait: Gait,
    /// A clip per model (by hash) played instead of the gait, or held
    /// still this many seconds in.
    pub clips: Vec<(u32, Option<f32>)>,
    /// Degrees each model is turned about the vertical.
    pub turn: f32,
}

/// Builds the scene on `backend` and points a camera at it.
/// Frames timed before a screenshot.
const FRAMES_TIMED: u32 = 60;

fn build_scene(
    backend: &mut dyn RenderBackend,
    install: Option<&Install>,
    mode: &Mode,
    double_sided: bool,
    sea_level: f32,
    mission: bool,
    hero: Hero,
) -> anyhow::Result<Game> {
    let mut scene = Scene::new(backend)?;
    scene.hero = hero;
    // The catalogue's vehicles are no mission's.
    scene.park = !mission;
    if double_sided {
        scene.materials.set_double_sided(backend, true)?;
    }
    if let Some(install) = install {
        match mode {
            Mode::Island(spawn) => {
                let settings = WorldSettings {
                    sea_level,
                    ..WorldSettings::default()
                };
                let start = if mission {
                    scene::MISSION_START
                } else {
                    scene::FREE_ROAM_START
                };
                scene.open_world(backend, install, settings, *spawn, start)?;
            }
            Mode::Models(show) => {
                scene.open(install)?;
                if let Err(e) = scene.load_models(backend, show) {
                    eprintln!("could not load models: {e:#}");
                }
            }
        }
    }
    let (eye, target) = scene.focus;
    let mut game = Game::new(scene, FlyCamera::looking_at(eye, target));
    if let (Some(install), Mode::Island(_)) = (install, mode) {
        // A state layer's people wait for the scripts to load it.
        game.scene.people.state_layers = Some(HashSet::new());
        // Free roam: the island's vehicles stand where the game's start-up
        // layers put them (with `--mission` the scripts' own layers do).
        if !mission {
            game.startup = Some(startup::Startup::start(install, game.scene.hero));
        }
    }
    Ok(game)
}

/// Hands her the weapons `--gun` names.
fn give_guns(game: &mut Game, guns: &[String]) {
    for name in guns {
        if !game.give(name) {
            eprintln!("--gun {name}: no hand weapon of that name in the templates");
        }
    }
}

/// What the HUD shows: the game's, with the mission's objectives and the
/// view their markers are placed with.
/// The game's own HUD, on the island with the game's files (unless
/// `--old-hud`), its scripts' player the hero `game` plays.
fn game_hud(
    install: Option<&Install>,
    mode: &Mode,
    old: bool,
    game: &Game,
) -> Option<gamehud::GameHud> {
    match (install, mode) {
        (Some(install), Mode::Island(_)) if !old => Some(gamehud::GameHud::start(
            install,
            game.scene.hero,
            game.combat.max_hp,
        )),
        _ => None,
    }
}

/// The game's loading screen from the install's `Loading.wad`, if it
/// reads.
fn loading_screen(install: Option<&Install>) -> Option<loading::LoadingScreen> {
    match loading::LoadingScreen::open(&install?.data) {
        Ok(screen) => {
            eprintln!(
                "loading screen: {} read, {} of {} hints",
                loading::MOVIE,
                screen.hints_read(),
                loading::HINTS.len()
            );
            Some(screen)
        }
        Err(e) => {
            eprintln!("loading screen: none ({e})");
            None
        }
    }
}

/// Writes the game to `path`, with her cash, fuel and support stock and
/// the scripts' own save data as the scripts hold them. Whether it was
/// written.
/// The world's aspect ratio in a `w` x `h` picture: the picture's, or
/// 4:3 with the video screen's Aspect Ratio at 4:3 (the world as on a
/// 4:3 screen, stretched over the picture).
fn view_aspect(window: &config::WindowConfig, w: u32, h: u32) -> f32 {
    if window.wide {
        w as f32 / h.max(1) as f32
    } else {
        4.0 / 3.0
    }
}

/// `--shell-click`'s place among the `--shell-key`s.
const CLICK: i32 = -1;

/// The game's pause menu's save slots go up to this; slot 0 is the
/// scripts' autosave.
const PAUSE_SLOTS: u32 = 20;

/// The slot of the newest save there is (what the front end's Continue
/// loads), by when its file was written.
fn newest_save() -> Option<u32> {
    (0..=PAUSE_SLOTS)
        .filter_map(|n| {
            let when = std::fs::metadata(pause_save_path(n))
                .and_then(|m| m.modified())
                .ok()?;
            Some((when, n))
        })
        .max()
        .map(|(_, n)| n)
}

/// Where the pause menu's save slot `n` is: the scripts' autosave for 0.
fn pause_save_path(n: u32) -> std::path::PathBuf {
    if n == 0 {
        save::SaveGame::slot_path(m2_game::api::AUTOSAVE)
    } else {
        save::SaveGame::slot_path(&format!("save{n}"))
    }
}

/// The saves there are, as the menus' save screens list them: each as its
/// hero's, `hero`'s if it doesn't say whose it is (saves from before).
fn pause_saves(hero: Hero) -> Vec<pause::SaveEntry> {
    (0..=PAUSE_SLOTS)
        .filter_map(|n| {
            let path = pause_save_path(n);
            let when = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            let s = save::SaveGame::read(&path).ok()?;
            let secs = when
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let pmc = s.pmc.unwrap_or_default();
            Some(pause::SaveEntry {
                number: n,
                timestamp: pause::timestamp(secs),
                character: match s.hero.unwrap_or(hero) {
                    Hero::Mattias => 1,
                    Hero::Chris => 2,
                    Hero::Jennifer => 3,
                },
                money: pmc.cash,
                fuel: pmc.fuel,
                played: String::new(),
                last_contract: String::new(),
                auto: n == 0,
                percent: 0.0,
            })
        })
        .collect()
}

fn write_save(game: &Game, mission: Option<&mission::MissionLink>, path: &std::path::Path) -> bool {
    let Some(mut s) = game.save() else {
        eprintln!("nothing to save until the island has loaded");
        return false;
    };
    s.pmc = mission.and_then(mission::MissionLink::pmc_save);
    s.scripts = mission.and_then(mission::MissionLink::script_save);
    match s.write(path) {
        Ok(()) => {
            eprintln!(
                "game saved to {} (cash {}, scripts' data {} bytes)",
                path.display(),
                s.pmc.as_ref().map_or(0.0, |p| p.cash),
                s.scripts.as_ref().map_or(0, String::len)
            );
            if let Some(c) = mission.and_then(mission::MissionLink::contracts) {
                eprintln!("game saved with {c}");
            }
            true
        }
        Err(e) => {
            eprintln!("could not save: {e:#}");
            false
        }
    }
}

/// The launch's arguments `args` with `--load save` in place of any
/// `--load` they had, and without `--load-at` (which would load again).
fn args_loading(args: impl IntoIterator<Item = String>, save: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        if a == "--load" || a == "--load-at" {
            args.next();
        } else {
            out.push(a);
        }
    }
    out.extend(["--load".to_owned(), save.display().to_string()]);
    out
}

/// The launch's arguments `args` to start again at the front end: without
/// any `--load`, with `--to-front-end`.
fn args_front_end(args: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        match a.as_str() {
            "--load" => {
                args.next();
            }
            "--to-front-end" => {}
            _ => out.push(a),
        }
    }
    out.push("--to-front-end".to_owned());
    out
}

/// Starts the alpha again at its front end, as the game goes back to its
/// main menu (the pause menu's Quit). The caller then closes and waits for
/// it ([`wait_for_successor`]).
fn relaunch_front_end() -> Option<std::process::Child> {
    let args = args_front_end(std::env::args().skip(1));
    match std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).args(&args).spawn())
    {
        Ok(child) => {
            eprintln!("quit: started again at the front end");
            Some(child)
        }
        Err(e) => {
            eprintln!("quit: could not start again: {e}");
            None
        }
    }
}

/// Starts the alpha again as it was launched, from the save at `save`
/// (`--load`): loading a save with the scripts' data starts the world
/// and the scripts afresh, as the game does. The caller then closes and
/// waits for it ([`wait_for_successor`]).
fn relaunch(save: &std::path::Path) -> Option<std::process::Child> {
    let args = args_loading(std::env::args().skip(1), save);
    match std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).args(&args).spawn())
    {
        Ok(child) => {
            eprintln!("load: started again with --load {}", save.display());
            Some(child)
        }
        Err(e) => {
            eprintln!("load: could not start again: {e}");
            None
        }
    }
}

fn hud_info(
    game: &Game,
    mission: Option<&mission::MissionLink>,
    fov: f32,
    aspect: f32,
) -> ui::HudInfo {
    let mut info = game.hud_info(mission.and_then(mission::MissionLink::strings));
    if let Some(m) = mission {
        info.mission = Some(m.view());
    }
    let view = game.view(fov, aspect);
    info.view_proj = Some(view.projection * view.view);
    info
}

/// Streams around the player (or camera) and draws a frame.
fn render(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    hud: Option<&mut ui::Hud>,
) -> anyhow::Result<()> {
    game.lens = (fov, aspect);
    let mut view = game.view(fov, aspect);
    if game.scene.world.is_some() {
        view.lighting.fog_density = 0.00025;
    }
    let focus = game.focus();
    spans::time("stream", || game.scene.update(backend, focus))?;
    let mut queue = DrawQueue::new();
    spans::time("draw", || -> anyhow::Result<()> {
        backend.begin_frame(&view)?;
        game.scene.draw(backend, &mut queue, &view, &game.combat)?;
        queue.flush(&view, backend)?;
        Ok(())
    })?;
    spans::time("present", || match hud {
        Some(hud) => backend.end_frame_with(&mut |t| hud.paint(t)),
        None => backend.end_frame(),
    })?;
    Ok(())
}

/// Renders one frame offscreen and writes it as a binary PPM. On the
/// island, once it has streamed in, the player lands and (with `gait`
/// walk, run or sprint) heads forward for three seconds first.
/// Plays `seconds` on, logging the traffic every five seconds.
/// `--riders`: seats the riders of the vehicles near her, as play does
/// every tick ([`hijack::Hijacks::seat`]), waiting up to 90 s for the
/// clips and their people's models, and logs how they came out. `take`:
/// then she takes the nearest seated driver's vehicle, as the use key
/// does, and the log says where whoever was put out is 3 s on.
fn riders_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    install: &Install,
    fov: f32,
    aspect: f32,
    take: bool,
) -> anyhow::Result<()> {
    let tick = TICK.as_secs_f32();
    let mut h = hijack::Hijacks::start(install, None);
    let mut input = Input::default();
    let waited = Instant::now();
    while h.loading() && waited.elapsed() < Duration::from_secs(120) {
        h.tick(game, &mut input, tick);
        game.step(Intent::default(), tick);
    }
    let waited = Instant::now();
    for i in 0.. {
        h.tick(game, &mut input, tick);
        game.step(Intent::default(), tick);
        h.seat(game, tick);
        // The scene takes what has loaded (people's models) as it draws.
        if i % 10 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
        let done = i > 60 && h.riders.no_kind == 0;
        if done || waited.elapsed() > Duration::from_secs(90) {
            break;
        }
    }
    // Beside the nearest one seated, looking at them, for the picture.
    let here = game.focus();
    let nearest = game
        .scene
        .riders
        .iter()
        .map(|r| {
            let side = r.frame.transform_vector3(m2_math::glam::Vec3::X);
            (r.frame.transform_point3(m2_math::glam::Vec3::ZERO), side)
        })
        .min_by(|a, b| a.0.distance(here).total_cmp(&b.0.distance(here)));
    if let (Some((at, side)), Some(p)) = (nearest, game.scene.player.as_mut()) {
        // Out from the driver's side (the seat's +x), level with them.
        let side = side.with_y(0.0).normalize_or(m2_math::glam::Vec3::X);
        let spot = at + side * 5.0 + m2_math::glam::Vec3::Y * 0.5;
        p.character.position = spot;
        p.character.velocity = m2_math::glam::Vec3::ZERO;
        p.landed = false;
        p.facing = (at.x - spot.x).atan2(at.z - spot.z);
        p.yaw = p.facing;
        eprintln!("riders: the picture from beside {:.1?}", at.to_array());
        for _ in 0..20 {
            game.step(Intent::default(), tick);
            h.seat(game, tick);
        }
    }
    eprintln!("riders: for the picture {:?}", h.riders);
    if take {
        take_for_screenshot(backend, game, &mut h, fov, aspect)?;
    }
    Ok(())
}

/// `--take`: she takes the vehicle of the nearest seated driver; 3 s on,
/// where those put out are and what they do.
fn take_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    h: &mut hijack::Hijacks,
    fov: f32,
    aspect: f32,
) -> anyhow::Result<()> {
    let tick = TICK.as_secs_f32();
    let here = game.focus();
    let driven = |v: &scene::ParkedVehicle| {
        v.crew
            .iter()
            .any(|(seat, _)| seat.to_ascii_lowercase().contains("driver"))
    };
    let pick = game
        .scene
        .riders
        .iter()
        .filter(|r| game.scene.vehicles.get(r.vehicle).is_some_and(driven))
        .map(|r| {
            (
                r.vehicle,
                r.frame.transform_point3(m2_math::glam::Vec3::ZERO),
            )
        })
        .min_by(|a, b| a.1.distance(here).total_cmp(&b.1.distance(here)));
    let Some((i, at)) = pick else {
        eprintln!("take: no seated driver to take a vehicle from");
        return Ok(());
    };
    let before = game.scene.people.npcs.len();
    if let Some(p) = game.scene.player.as_mut() {
        p.character.position = at;
        p.driving = Some(i);
        p.landed = true;
    }
    game.take_from_driver(i);
    let out = game.scene.people.npcs.len() - before;
    eprintln!(
        "take: the {} at {:.1?}: {out} put out",
        game.scene.vehicles[i].spec.label,
        at.to_array()
    );
    for t in 0..180 {
        game.step(Intent::default(), tick);
        h.seat(game, tick);
        if t % 10 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    let seated = game
        .scene
        .riders
        .iter()
        .filter(|r| r.prop.is_none() && r.vehicle == i)
        .count();
    for n in &game.scene.people.npcs[before..] {
        let p = n.character.position;
        eprintln!(
            "take: put out, 3 s on: {:.1} m from the seat at {:.1?}, {}, alive {}, landed {}",
            p.distance(at),
            p.to_array(),
            n.doing(),
            n.alive,
            n.landed
        );
    }
    eprintln!("take: riders still drawn in it: {seated}");
    Ok(())
}

fn traffic_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    seconds: f32,
) -> anyhow::Result<()> {
    let tick = TICK.as_secs_f32();
    let mut t = 0.0;
    let mut next_log = 0.0;
    // What lives near her, every 2 s.
    game.census.get_or_insert((2.0, 0.0, 2.0));
    while t <= seconds {
        if t >= next_log {
            next_log += 5.0;
            if let Some(world) = &game.scene.world {
                let focus = game.focus();
                eprintln!("traffic at {t:.0} s:");
                for line in game
                    .traffic
                    .report(&world.physics, &game.scene.vehicles, focus)
                {
                    eprintln!("{line}");
                }
                for line in
                    game.roads
                        .report(&world.physics, &game.scene.vehicles, &world.index.roads)
                {
                    eprintln!("{line}");
                }
            }
        }
        game.step(Intent::default(), tick);
        render(backend, game, fov, aspect, None)?;
        t += tick;
    }
    Ok(())
}

fn screenshot(
    config: &Config,
    install: Option<&Install>,
    mode: &Mode,
    options: &Options,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let (w, h) = (config.window.width, config.window.height);
    let mut backend = WgpuBackend::headless(w, h)?;
    eprintln!("adapter: {:?}", backend.adapter_summary());
    let mut game = build_scene(
        &mut backend,
        install,
        mode,
        options.double_sided,
        config.world.sea_level,
        options.mission,
        config.hero,
    )?;
    game.scene.people.smarter = config.enhanced.smarter_ai;
    give_guns(&mut game, &options.guns);
    if options.fly {
        game.on_foot = false;
    }
    let fov = config.camera.fov;
    let mut aspect = w as f32 / h as f32;
    let tick = TICK.as_secs_f32();
    // Characters a little way into their clip, mid-stride when moving.
    game.scene.tick(0.4);
    render(&mut backend, &mut game, fov, aspect, None)?;
    // Let the island stream in before the picture is taken.
    let started = Instant::now();
    while let Some(world) = &game.scene.world {
        let stats = world.stats();
        let waiting = game
            .scene
            .player
            .as_ref()
            .is_some_and(|p| game.on_foot && !p.landed);
        if (stats.pending == 0 && !waiting) || started.elapsed() > Duration::from_secs(180) {
            eprintln!("{stats:?} after {:.1} s", started.elapsed().as_secs_f32());
            eprintln!("atmosphere: {:?}", world.index.atmosphere);
            for r in &world.index.atmosphere_regions {
                let names: Vec<String> = r
                    .settings
                    .iter()
                    .map(|(n, a)| {
                        format!(
                            "{n:08x} light time {} intensity {}",
                            a.light_time[0], a.light_intensity
                        )
                    })
                    .collect();
                eprintln!(
                    "atmosphere region {} {:08x}: {} corners, {:.0} m2, {}",
                    r.name,
                    r.entity,
                    r.outline.len(),
                    r.area,
                    names.join("; ")
                );
            }
            break;
        }
        game.step(Intent::default(), tick);
        render(&mut backend, &mut game, fov, aspect, None)?;
    }
    // And for the vehicles and people, and the people to find their feet,
    // and for the vehicles to be parked by her (a look for a spot can take
    // a while: not timed). The island's own vehicles get half a minute, as
    // one may fail, and the props within reach two minutes: the frames
    // are timed as play goes, not as the island first loads.
    while game.scene.world.is_some()
        && (game.scene.vehicles_loading()
            || (game.on_foot
                && game.scene.player.as_ref().is_some_and(|p| p.landed)
                && !game.parking_done())
            || (game.island_vehicles_loading() && started.elapsed() < Duration::from_secs(30))
            || (!game.scene.props.settled() && started.elapsed() < Duration::from_secs(120)))
        && started.elapsed() < Duration::from_secs(300)
    {
        game.step(Intent::default(), tick);
        render(&mut backend, &mut game, fov, aspect, None)?;
    }
    eprintln!(
        "props within reach {} after {:.1} s",
        if game.scene.props.settled() {
            "loaded"
        } else {
            "still loading"
        },
        started.elapsed().as_secs_f32()
    );
    if let Some(setup) = &options.coop {
        let walk = options.gait.filter(|_| game.on_foot);
        return coop::offscreen(
            &mut backend,
            &mut game,
            setup,
            options.coop_seconds,
            walk,
            fov,
            aspect,
            out,
        );
    }
    // The last frames are timed: the frame rate here, for checking.
    // Where the time goes: the tick, streaming, building the draws, and
    // submitting the frame.
    let timed = Instant::now();
    let mut parts = [0.0f32; 4];
    game.scene.streamed = Default::default();
    for _ in 0..FRAMES_TIMED {
        let mut lap = Instant::now();
        let mut split = |i: usize, lap: &mut Instant| {
            parts[i] += lap.elapsed().as_secs_f32();
            *lap = Instant::now();
        };
        game.step(Intent::default(), tick);
        split(0, &mut lap);
        let mut view = game.view(fov, aspect);
        view.lighting.fog_density = 0.00025;
        let focus = game.focus();
        game.scene.update(&mut backend, focus)?;
        split(1, &mut lap);
        let mut queue = DrawQueue::new();
        backend.begin_frame(&view)?;
        game.scene
            .draw(&mut backend, &mut queue, &view, &game.combat)?;
        queue.flush(&view, &mut backend)?;
        split(2, &mut lap);
        backend.end_frame()?;
        split(3, &mut lap);
    }
    let each = timed.elapsed().as_secs_f32() / FRAMES_TIMED as f32;
    let ms = parts.map(|p| p * 1000.0 / FRAMES_TIMED as f32);
    let streamed = game
        .scene
        .streamed
        .map(|d| d.as_secs_f32() * 1000.0 / FRAMES_TIMED as f32);
    eprintln!(
        "frames: {:.2} ms each ({:.0} fps) over {FRAMES_TIMED}, with a tick each: \
         tick {:.2}, streaming {:.2} (the island {:.2}, vehicles and people {:.2}, \
         props {:.2}), draws {:.2}, submit {:.2} ms",
        each * 1000.0,
        1.0 / each.max(1e-6),
        ms[0],
        ms[1],
        streamed[0],
        streamed[1],
        streamed[2],
        ms[2],
        ms[3]
    );
    eprintln!("props: {}", game.scene.props.counts());
    let people = &game.scene.people;
    eprintln!(
        "people: {} kinds, {} about ({} soldiers, {} story characters, {} patrolling, {} landed) after {:.1} s",
        people.kinds.len(),
        people.npcs.len(),
        people.npcs.iter().filter(|n| n.soldier).count(),
        people.npcs.iter().filter(|n| n.held).count(),
        people.npcs.iter().filter(|n| n.patrolling()).count(),
        people.npcs.iter().filter(|n| n.landed).count(),
        started.elapsed().as_secs_f32()
    );
    if let Some(world) = &game.scene.world {
        // Where the soldiers' spawns are, for checking placement.
        let focus = game.focus();
        let spawns = &world.index.spawns;
        let mut near: Vec<(f32, &m2_engine::world::SpawnPoint)> = spawns
            .iter()
            .map(|s| (s.position.distance(focus), s))
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        eprintln!(
            "spawns: {} ({} named for a soldier); nearest: {}",
            spawns.len(),
            spawns
                .iter()
                .filter(|s| people.kind_for(&s.name).is_some())
                .count(),
            near.iter()
                .take(6)
                .map(|(d, s)| format!("{:?} {d:.0} m", s.name))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for (d, s) in near.iter().take(options.list_spawns) {
            eprintln!(
                "  spawn {:08x} {:?} {d:.0} m at {:.0}, template {:?}, layer {} ({}), {} route points, {}{}{}",
                s.entity,
                s.name,
                s.position,
                s.template,
                match (s.layer, s.static_layer) {
                    (Some(l), true) => format!("static {l:08x}"),
                    (Some(l), false) => format!("{l:08x}"),
                    (None, _) => "none".to_owned(),
                },
                if people.layer_in(s.layer) {
                    "in"
                } else {
                    "out"
                },
                s.route.len(),
                people.stands_as(s),
                if people.mission_soldiers.contains(&s.entity) {
                    ", a mission's target"
                } else {
                    ""
                },
                if people.npcs.iter().any(|n| n.spawn == Some(s.entity)) {
                    ", standing"
                } else {
                    ""
                }
            );
        }
    }
    if let Some(seconds) = options.traffic {
        traffic_for_screenshot(&mut backend, &mut game, fov, aspect, seconds)?;
    }
    if options.riders
        && let Some(install) = install
    {
        riders_for_screenshot(&mut backend, &mut game, install, fov, aspect, options.take)?;
    }
    game.scene.hide_fx = options.no_fx;
    if options.old_fx {
        game.scene.particles = None;
    }
    if options.probe {
        probe_for_screenshot(&game);
    }
    let mut link = None;
    if options.mission
        && let Some(install) = install
    {
        link = Some(mission_for_screenshot(
            &mut backend,
            &mut game,
            install,
            options,
            fov,
            aspect,
            out,
        )?);
    } else if let Some(s) = &options.load {
        // Without the scripts only the world's part loads.
        game.load(s.clone());
    }
    if let Some(path) = &options.save_to {
        write_save(&game, link.as_ref(), path);
    }
    if options.gates {
        if let Some(install) = install {
            eprintln!(
                "sound: {} of the fight's 6 sounds found",
                sound::fight_sounds_found(install)
            );
        }
        gates_for_screenshot(&mut backend, &mut game, fov, aspect, out)?;
    }
    if let Some(stage) = options
        .satellite
        .filter(|_| game.on_foot && game.scene.player.is_some())
    {
        satellite_for_screenshot(
            &mut backend,
            &mut game,
            fov,
            aspect,
            stage,
            options.fight_wait,
        )?;
    } else if options.fight && game.on_foot && game.scene.player.is_some() {
        fight_for_screenshot(&mut backend, &mut game, fov, aspect, options.fight_wait)?;
    } else if let Some(rockets) = options
        .demolish
        .filter(|_| game.on_foot && game.scene.player.is_some())
    {
        demolish_for_screenshot(&mut backend, &mut game, fov, aspect, rockets, out)?;
    } else if options.blast && game.on_foot && game.scene.player.is_some() {
        blast_for_screenshot(&mut backend, &mut game, fov, aspect, options.fight_wait)?;
    } else if options.drive && game.on_foot && game.scene.player.is_some() {
        drive_for_screenshot(&mut backend, &mut game, fov, aspect, options, install)?;
    } else if game.on_foot && game.scene.player.is_some() {
        let gait = options.gait.unwrap_or(Gait::Idle);
        let intent = Intent {
            movement: if gait == Gait::Idle {
                Default::default()
            } else {
                m2_math::glam::Vec2::Y
            },
            walk: gait == Gait::Walk,
            sprint: gait == Gait::Sprint,
            ..Intent::default()
        };
        let from = game.scene.player.as_ref().map(|p| p.character.position);
        // A jump, throw or bash starts this many ticks before the picture.
        let lead = (options.lead.unwrap_or(0.2) * 60.0)
            .round()
            .clamp(1.0, 179.0) as usize;
        // The crate's grenade launcher, as she takes it up: three clips.
        if options.weapon == Some(combat::Weapon::GrenadeLauncher) {
            game.combat
                .equip(combat::Weapon::GrenadeLauncher, 3 * combat::LAUNCHER_CLIP);
        }
        // A launcher on her back, the rifle up.
        if let Some(w) = options.slung {
            game.combat.equip(w, 3 * combat::LAUNCHER_CLIP);
            game.combat.weapon = options.weapon.unwrap_or(combat::Weapon::Rifle);
        }
        // `--reload`: rounds spent first, so the clip has room.
        if options.reload {
            let r = &mut game.combat.rifle_rounds;
            r.clip = r.clip.saturating_sub(5);
        }
        // `--walk-seconds`: she goes on this long (3 s by default).
        let ticks = options
            .walk_seconds
            .map_or(180, |s| (s * 60.0).round() as usize);
        // `--npc-swim`: the soldier put in the water.
        let (mut swimmer, mut swim_gave_up): (Option<u32>, bool) = (None, false);
        for i in 0..ticks {
            if let (Some(face), Some(p)) = (options.face, game.scene.player.as_mut()) {
                p.yaw = face;
                if i == 0 {
                    p.facing = face;
                }
            }
            if let (Some(pitch), Some(p)) = (options.pitch, game.scene.player.as_mut()) {
                p.pitch = pitch;
            }
            // The bash clip asked for, with its blow's time.
            if let (Some(want), Some(p)) = (options.bash_clip, game.scene.player.as_mut())
                && i == 0
            {
                let found = p.model.as_ref().and_then(|(_, rig)| {
                    let c = rig.clips.iter().position(|c| c.hash == want)?;
                    let at = rig.clips[c]
                        .clip
                        .events
                        .iter()
                        .find(|e| e.name.eq_ignore_ascii_case("meleebash"))
                        .map_or(0.37, |e| e.time);
                    Some((c, at))
                });
                match found {
                    Some(clip) => p.clips.bash = Some(clip),
                    None => eprintln!("--bash-clip {want:08x}: she has no such clip"),
                }
            }
            // A jump takes the picture on the way up (by default), a
            // throw with the grenade still in her hand; aiming, the
            // weapon comes up for the last second.
            let jump = (options.jump && i == ticks - lead) || (options.jump_spam && i % 6 == 0);
            // With --jump-with-enter too, the scripts take her controls a
            // moment into the jump, as the contract's end does when its
            // fanfare is closed with the pad's A (playtest 8: she hung in
            // the air till the screen went black); where she is after.
            if options.jump && options.jump_with_enter && i > ticks - lead {
                if i == ticks - lead + 5 {
                    game.input_locked = true;
                    eprintln!("held: her controls taken 5 ticks into the jump");
                }
                if i % 6 == 0
                    && let Some(p) = &game.scene.player
                {
                    eprintln!(
                        "held: tick {i}, she is at {:.2}, landed {}, grounded {}, rising {:.2} m/s",
                        p.character.position,
                        p.landed,
                        p.character.grounded,
                        p.character.velocity.y
                    );
                }
            }
            let trigger = combat::Trigger {
                aim: options.aim && i + 60 >= ticks,
                // The last shot's flash is still up for the picture.
                fire: options.fire && i + 3 == ticks,
                throw: options.throw && i == ticks - lead,
                melee: options.bash && i == ticks - lead,
                reload: options.reload && i == ticks - lead,
                select: options.weapon.filter(|_| i == 0).or_else(|| {
                    (options.switch && i == ticks - lead).then_some(match game.combat.weapon {
                        combat::Weapon::Rifle => combat::Weapon::Rpg,
                        _ => combat::Weapon::Rifle,
                    })
                }),
                ..combat::Trigger::default()
            };
            // `--npc-swim`: once a soldier is in play, the nearest is put in
            // the nearest water out of his depth (2 m deep) from `ahead`
            // metres out, at its surface, and she 8 m back from him.
            if let (Some(ahead), None, false) = (options.npc_swim, swimmer, swim_gave_up)
                && game
                    .scene
                    .people
                    .npcs
                    .iter()
                    .any(|n| n.soldier && n.alive && n.landed)
                && let (Some(p), Some(w)) = (&game.scene.player, &game.scene.world)
            {
                let her = p.character.position;
                let deep = |to: m2_math::glam::Vec3| {
                    let s = w.physics.water_at(to.x, to.z)?;
                    let (d, _) =
                        w.physics
                            .cast_ray(to.with_y(s + 30.0), -m2_math::glam::Vec3::Y, 400.0)?;
                    (d - 30.0 > 2.0).then_some(s)
                };
                let found = (0..40).map(|r| ahead + r as f32 * 10.0).find_map(|r| {
                    (0..16).find_map(|k| {
                        let a = p.facing + k as f32 * std::f32::consts::TAU / 16.0;
                        let way = m2_math::glam::Vec3::new(a.sin(), 0.0, a.cos());
                        let to = her + way * r;
                        deep(to).map(|s| (to.with_y(s), way))
                    })
                });
                match found {
                    Some((to, way)) => {
                        let back = her + (to - her).with_y(0.0) - way * 8.0;
                        let n = game
                            .scene
                            .people
                            .npcs
                            .iter_mut()
                            .filter(|n| n.soldier && n.alive && n.landed)
                            .min_by(|a, b| {
                                let d = |n: &npc::Npc| n.character.position.distance_squared(her);
                                d(a).total_cmp(&d(b))
                            })
                            .expect("a soldier is in play");
                        eprintln!(
                            "npc-swim: {:08x} put from {:.1} to {to:.1}, {:.0} m from where she was; she goes to {:.1}",
                            n.spawn.unwrap_or(0),
                            n.character.position,
                            to.distance(her),
                            back
                        );
                        n.character.position = to;
                        n.character.velocity = m2_math::glam::Vec3::ZERO;
                        n.alert = false;
                        swimmer = n.spawn;
                        if let Some(p) = game.scene.player.as_mut() {
                            p.drop_at(back + m2_math::glam::Vec3::Y * 2.0);
                            p.facing = way.x.atan2(way.z);
                            p.yaw = p.facing;
                        }
                    }
                    None => {
                        eprintln!(
                            "npc-swim: no water out of his depth within {:.0} m",
                            ahead + 390.0
                        );
                        swim_gave_up = true;
                    }
                }
            }
            if options.pickup
                && i == ticks - lead
                && let Some(p) = game.scene.player.as_mut()
                && !p.start_pickup()
            {
                eprintln!("--pickup: she has no pickup clip, or is busy");
            }
            let mut intent = Intent {
                jump,
                hold_jump: jump,
                trigger,
                ..intent
            };
            // Her gun's lens and scope as she aims (as play does).
            game.aim_lens(&mut intent, 0, tick);
            game.step(intent, tick);
            // With `--walk-seconds`, where she is each second: how deep the
            // sea is over her feet, and whether she swims.
            if options.walk_seconds.is_some()
                && i % 60 == 59
                && let (Some(p), Some(w)) = (&game.scene.player, &game.scene.world)
            {
                let at = p.character.position;
                eprintln!(
                    "walk: {:.0} s at {at:.1}, sea {} over her feet, {}",
                    (i + 1) as f32 / 60.0,
                    w.physics
                        .water()
                        .map_or("none".to_owned(), |s| format!("{:.2} m", s - at.y)),
                    if p.swimming() {
                        "swimming"
                    } else {
                        "on her feet"
                    }
                );
            }
            // With `--npc-swim`, each second: where the soldier put in the
            // water is, how deep it is over his feet, and what he does.
            if options.npc_swim.is_some()
                && i % 60 == 59
                && let (Some(p), Some(w)) = (&game.scene.player, &game.scene.world)
                && let Some(n) = game
                    .scene
                    .people
                    .npcs
                    .iter()
                    .find(|n| swimmer.is_some() && n.spawn == swimmer)
            {
                let at = n.character.position;
                let water = w.physics.water_at(at.x, at.z);
                let ground = w
                    .physics
                    .cast_ray(
                        at.with_y(at.y.max(water.unwrap_or(at.y)) + 1.0),
                        -m2_math::glam::Vec3::Y,
                        400.0,
                    )
                    .map(|(d, _)| at.y.max(water.unwrap_or(at.y)) + 1.0 - d);
                let clip = n
                    .rig
                    .animator
                    .current()
                    .map_or(0, |(c, _)| n.rig.clips[c].hash);
                eprintln!(
                    "npc-swim: {:.0} s {:08x} at {at:.2}, water {} over his feet, ground {} under the surface, {}, clip {clip:08x}, {:.1} m from her",
                    (i + 1) as f32 / 60.0,
                    n.spawn.unwrap_or(0),
                    water.map_or("none".to_owned(), |s| format!("{:.2} m", s - at.y)),
                    match (water, ground) {
                        (Some(s), Some(g)) => format!("{:.2} m", s - g),
                        _ => "-".to_owned(),
                    },
                    if n.swimming() {
                        "swimming"
                    } else {
                        "on his feet"
                    },
                    at.distance(p.character.position)
                );
            }
            if i % 30 == 0 {
                // Keep streaming around her as she goes.
                render(&mut backend, &mut game, fov, aspect, None)?;
            }
        }
        if let (Some(p), Some(from)) = (&game.scene.player, from) {
            let hash = |c: usize| p.model.as_ref().map_or(0, |(_, r)| r.clips[c].hash);
            eprintln!(
                "player: {:?} from {from:.2} to {:.2}, grounded {}, clip {:?}, upper {:?}, throwing {:?}, grenade in hand {}, grenades out {}, bashing {:?}",
                gait,
                p.character.position,
                p.character.grounded,
                p.model
                    .as_ref()
                    .and_then(|(_, r)| r.animator.current())
                    .map(|(c, t)| format!("{:08x} at {t:.2} s", hash(c))),
                p.model
                    .as_ref()
                    .and_then(|(_, r)| r.animator.upper())
                    .map(|c| format!("{:08x}", hash(c))),
                p.throwing,
                game.combat.grenade_in_hand(),
                game.combat
                    .projectiles()
                    .filter(|(what, ..)| *what == combat::Flying::Grenade)
                    .count(),
                p.bashing,
            );
        }
    }
    if options.closeup
        && let Some(p) = &game.scene.player
    {
        // From her right and a little ahead, at chest height.
        let chest = p.character.position + m2_math::glam::Vec3::Y * 1.25;
        let ahead = m2_math::glam::Vec3::new(p.facing.sin(), 0.0, p.facing.cos());
        let right = m2_math::glam::Vec3::new(-ahead.z, 0.0, ahead.x);
        game.on_foot = false;
        game.camera = FlyCamera::looking_at(chest + right * 1.3 + ahead * 0.9, chest);
        eprintln!("closeup: chest {chest:.2}, facing {ahead:.2}");
        if let Some(line) = describe_hold(&game) {
            eprintln!("closeup: {line}");
        }
    }
    if options.behind
        && let Some(p) = &game.scene.player
    {
        // From behind her and a little to her right, at shoulder height.
        let shoulders = p.character.position + m2_math::glam::Vec3::Y * 1.4;
        let ahead = m2_math::glam::Vec3::new(p.facing.sin(), 0.0, p.facing.cos());
        let right = m2_math::glam::Vec3::new(-ahead.z, 0.0, ahead.x);
        game.on_foot = false;
        game.camera = FlyCamera::looking_at(shoulders - ahead * 1.6 + right * 0.5, shoulders);
        for line in describe_slung(&game) {
            eprintln!("behind: {line}");
        }
    }
    if let (Some(reach), Some(link)) = (options.mission_look, &link) {
        look_at_marker(&mut backend, &mut game, link, reach, fov, aspect)?;
    }
    if let Some(reach) = options.look_pickup {
        look_at_pickup(&mut game, reach);
    }
    if let Some(p) = game.scene.player.as_ref().filter(|_| game.on_foot) {
        // Hits from around her, as the camera sees it: 0 ahead, 90 right.
        let at = p.character.position + m2_math::glam::Vec3::Y;
        let look = FlyCamera {
            yaw: game.view_heading(),
            ..game.camera
        };
        let ahead = look.forward().with_y(0.0).normalize_or_zero();
        let right = look.right();
        for d in &options.hit_from {
            let (s, c) = d.to_radians().sin_cos();
            game.combat
                .hurt_from(15.0, at + (ahead * c + right * s) * 10.0);
        }
    }
    if let Some(zoom) = options.binoculars
        && let Some(p) = game.scene.player.as_mut()
    {
        let mut b = binoculars::Binoculars::default();
        b.step(i32::from(zoom) - i32::from(b.zoom));
        p.binoculars = Some(b);
    }
    if options.pda {
        game.support.pda = true;
    }
    // The picture, with the HUD (the game's, once its scripts have
    // started and run two seconds, for its boxes to have opened and the
    // gun box to have switched; half of one for hits, whose arcs fade).
    let mut hud = ui::Hud::new();
    hud.game = game_hud(install, mode, options.old_hud, &game);
    hud.loading = loading_screen(install);
    let mut settings = config.clone();
    let mut info = hud_info(&game, link.as_ref(), fov, aspect);
    if hud.game.is_some() {
        // The video screen as for a window of the picture's size, its
        // lists the primary monitor's modes.
        let video = pause::Video {
            width: w,
            height: h,
            fullscreen: None,
        };
        let modes = if options.pause {
            display::primary_modes()
        } else {
            Vec::new()
        };
        if options.pause {
            eprintln!("pause: the monitor offers {} modes", modes.len());
        }
        let mut state = gamehud::HudState::of(&mut game);
        state.pause = options.pause;
        state.pause_settings.profile = game.scene.hero.full_name().into();
        state.pause_settings.video = video;
        state.pause_settings.modes = modes.clone();
        info.game = Some(state);
        let seconds = options
            .hud_seconds
            .unwrap_or(if options.hit_from.is_empty() {
                2.0
            } else {
                0.5
            });
        hud.settle(
            &info,
            (w, h),
            seconds,
            (&options.pda_keys, &options.pda_clicks),
            options.pause,
            Duration::from_secs(120),
        );
        let mut state = gamehud::HudState::of(&mut game);
        state.pause = options.pause;
        state.pause_settings.profile = game.scene.hero.full_name().into();
        state.pause_settings.video = video;
        state.pause_settings.modes = modes;
        info.game = Some(state);
        // What the game's pause menu asked of play, for the log.
        if let Some(g) = hud.game.as_mut() {
            let news = g.take_pause_news();
            if let Some(v) = news.video.filter(|&v| v != video) {
                // Offscreen there is no window to put into it.
                eprintln!(
                    "pause: video kept: {}, {} (a window would now be {})",
                    v.resolution_text(),
                    v.refresh_text(),
                    match v.fullscreen {
                        Some(_) => "fullscreen at that mode",
                        None => "resized to it",
                    }
                );
            }
            // What play would do with it: the settings kept go into the
            // settings file and the picture; Quit goes back to the front
            // end.
            if let Some(s) = &news.settings {
                settings.keep_menu_settings(s);
                game.picture.brightness = s.brightness;
                game.set_view_distance(s.view_distance);
                aspect = view_aspect(&settings.window, w, h);
                eprintln!(
                    "pause: settings kept: brightness {}, view distance {}, wide {}, subtitles {}",
                    s.brightness, s.view_distance, s.wide, s.subtitles
                );
            }
            if news.quit {
                eprintln!(
                    "pause: Quit: the alpha would start again with {:?}",
                    args_front_end(std::env::args().skip(1))
                );
            }
            if news != pause::News::default() {
                eprintln!("pause news: {news:?}");
            }
        }
    }
    // The loading screen as at the start (`--loading-screen`): up from
    // the first frame, run at 30 fps. `M2_LOADING_HINT=n` shows hint `n`.
    let mut time = 1.0;
    if let Some(seconds) = options.loading_seconds {
        info.loading = Some("--loading-screen".into());
        if let (Some(screen), Some(n)) = (
            hud.loading.as_mut(),
            std::env::var("M2_LOADING_HINT")
                .ok()
                .and_then(|v| v.parse().ok()),
        ) {
            screen.cycle.show_next(n);
        }
        let frames = (seconds * 30.0).round() as u32;
        for i in 0..frames {
            let t = f64::from(i) / 30.0;
            let _ = hud.build(ui::headless_input(w, h, t), &info, &mut settings);
        }
        time = f64::from(frames) / 30.0;
    }
    // The wipes (`--loading-wipe`): a frame without the screen first, so
    // it comes up over the world, not over black. `M2_WIPE_EVERY` also
    // writes each frame on the way (`<picture>.fNNN.ppm`).
    if let Some((up, out_seconds)) = options.loading_wipe {
        let every = std::env::var_os("M2_WIPE_EVERY").is_some();
        info.loading = None;
        let _ = hud.build(ui::headless_input(w, h, 0.0), &info, &mut settings);
        info.loading = Some("--loading-wipe".into());
        let frames_up = (up * 30.0).round() as u32;
        let frames = frames_up + out_seconds.map_or(0, |s| (s * 30.0).round() as u32);
        for i in 1..=frames {
            if i == frames_up + 1 {
                info.loading = None;
            }
            let _ = hud.build(
                ui::headless_input(w, h, f64::from(i) / 30.0),
                &info,
                &mut settings,
            );
            if every {
                render(&mut backend, &mut game, fov, aspect, Some(&mut hud))?;
                write_ppm(&backend, &out.with_extension(format!("f{i:03}.ppm")))?;
            }
        }
        time = f64::from(frames + 1) / 30.0;
    }
    if let Some(tab) = options.cheat_menu {
        let screen = cheats::CheatScreen::read(&game, link.as_ref());
        eprintln!("cheat menu: {}, the {} page", screen.counts(), tab.name());
        hud.cheats = Some(cheats::CheatScreen {
            tab,
            plain: true,
            ..screen
        });
        hud.menu = true;
        // The menu alone: no loading screen over it.
        hud.loading = None;
        info.loading = None;
        // A new egui window sizes itself on its first frame and fades in
        // over the next few.
        for _ in 0..10 {
            let _ = hud.build(ui::headless_input(w, h, time), &info, &mut settings);
            time += 0.1;
        }
    }
    let _ = hud.build(ui::headless_input(w, h, time), &info, &mut settings);
    render(&mut backend, &mut game, fov, aspect, Some(&mut hud))?;
    write_ppm(&backend, out)?;
    eprintln!("wrote {}", out.display());
    // The same frame without the HUD's fades, to compare (`M2_PAINT_AB`).
    if std::env::var_os("M2_PAINT_AB").is_some()
        && let Some(g) = hud.game.as_mut()
    {
        g.skip_covers = true;
        let _ = hud.build(ui::headless_input(w, h, 1.0), &info, &mut settings);
        render(&mut backend, &mut game, fov, aspect, Some(&mut hud))?;
        let plain = out.with_extension("nocover.ppm");
        write_ppm(&backend, &plain)?;
        eprintln!("wrote {}", plain.display());
    }
    Ok(())
}

/// Puts the camera `reach` metres off the pickup lying nearest her, a
/// little above it and looking at it, and says which it is and whether
/// its model has loaded.
fn look_at_pickup(game: &mut Game, reach: f32) {
    let Some(her) = game.scene.player.as_ref().map(|p| p.character.position) else {
        return;
    };
    let flights = &game.scene.flights;
    let Some(p) = flights
        .pickups()
        .iter()
        .min_by(|a, b| a.at.distance(her).total_cmp(&b.at.distance(her)))
    else {
        eprintln!("look-pickup: nothing lies to be taken");
        return;
    };
    eprintln!(
        "look-pickup: {:#x} at {:.1}, {:.1} m from her, model {:?} {}",
        p.what.template,
        p.at,
        p.at.distance(her),
        p.what.model,
        if p.what.model.is_some_and(|m| flights.has_model(m)) {
            "loaded"
        } else {
            "not loaded"
        }
    );
    let at = p.at + m2_math::glam::Vec3::Y * 0.2;
    // Who lies or stands by it (the soldier who left it).
    if let Some(n) = game
        .scene
        .people
        .npcs
        .iter()
        .filter(|n| n.character.position.distance(p.at) < 3.0)
        .min_by(|a, b| {
            a.character
                .position
                .distance(p.at)
                .total_cmp(&b.character.position.distance(p.at))
        })
    {
        eprintln!(
            "look-pickup: a person {:.1} m away, {}, {}",
            n.character.position.distance(p.at),
            if n.alive { "alive" } else { "dead" },
            if n.ragdoll.is_some() {
                "limp"
            } else {
                "not limp"
            }
        );
    }
    // From above, so that a body by it doesn't hide it.
    let back = (her - at).with_y(0.0).normalize_or(m2_math::glam::Vec3::Z);
    game.on_foot = false;
    game.camera =
        FlyCamera::looking_at(at + back * reach * 0.5 + m2_math::glam::Vec3::Y * reach, at);
}

/// What stands where a bomb fell (`at`, x and z), as its pictures are
/// taken: the state layers' models near it and how each stands (a gate
/// the bomb broke: its pieces, how far the rest has sunk, what is drawn
/// of it), and the island's own placed models whose boxes cover the spot
/// (round 99's broken gates still showed an arch and doors at 8 s, when
/// the gate's own wreck is drawn no more).
fn describe_bombed(game: &Game, at: m2_math::glam::Vec2, fov: f32, aspect: f32) {
    let Some(world) = &game.scene.world else {
        return;
    };
    let props = &game.scene.props;
    let mut near: Vec<(f32, u32)> = props
        .shown()
        .map(|(e, p)| {
            (
                p.position
                    .with_y(0.0)
                    .distance(m2_math::glam::Vec3::new(at.x, 0.0, at.y)),
                e,
            )
        })
        .filter(|&(d, _)| d < BOMBED_REACH)
        .collect();
    near.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (d, e) in near.iter().take(4) {
        if let Some(s) = props.describe(&world.physics, *e) {
            eprintln!("  prop {d:.1} m from the bomb: {s}");
        }
    }
    let spot = near
        .first()
        .and_then(|&(_, e)| props.centre(e))
        .unwrap_or(m2_math::glam::Vec3::new(at.x, game.focus().y, at.y));
    let eye = game.view(fov, aspect).eye();
    let over: Vec<String> = world
        .covering(spot, eye, 2.0)
        .into_iter()
        .map(|(model, place, size)| {
            format!(
                "{model:08x} at {place:.1}, {:.1}x{:.1}x{:.1} m",
                size.x, size.y, size.z
            )
        })
        .collect();
    eprintln!(
        "  the island's models drawn over {spot:.1}: {}",
        if over.is_empty() {
            "none".to_owned()
        } else {
            over.join("; ")
        }
    );
}

/// How far from where a bomb fell [`describe_bombed`] looks for the state
/// layers' models (metres): a gate's middle, as far as from one end of it.
const BOMBED_REACH: f32 = 40.0;

/// The mission's gates' model (`_jungle_wallgate`'s), which the gate test
/// brings down wherever a state layer places it.
const GATE_MODEL: u32 = 0x1923_143c;

/// How fast the gate test throws a gate's pieces each time it brings it
/// down, and whether everything else in its box is taken down first: as
/// in play first (and pictured), then slower and faster, to see how the
/// look depends on it, then as in play alone (and pictured).
const GATE_TRIES: [(f32, bool); 4] = [(1.0, false), (0.6, false), (1.6, false), (1.0, true)];

/// How much of another model's box has to be inside a gate's for the
/// gate test's last try to take it down.
const GATE_ALONE_SHARE: f32 = 0.5;

/// How many of the other models in a gate's box the gate test lists.
const GATE_LISTED: usize = 12;

/// How far (metres) a gate's piece may have gone and still be listed as
/// held, with what it touches: round 101's gatehouse pieces went 1 to 2 m.
const GATE_HELD: f32 = 2.5;

/// With `--gates`: brings down each of the mission's gates (the state
/// layers' [`GATE_MODEL`]s) as the satellite bomb does, its blast on the
/// middle of the gate's top, once for each of [`GATE_TRIES`], logging how
/// its pieces set off (`gate N, try T: sets off: ...`) and where they are
/// 2 s and 8 s on. First it lists what else stands in the gate's box (the
/// state layers' models, with the layer that put each up, and every
/// static collider) and pictures the place without the gate
/// (`<out>-gateN-without.ppm`): round 101's gates still showed whole walls
/// either side 8 s after the bomb, where the gate has no pieces, and its
/// gatehouse's pieces were held where they started. The first time,
/// pictures from 50 m south of it (as the mission comes at it) of the rest
/// of it alone and its pieces alone as it breaks (`-gateN-rest`,
/// `-gateN-pieces`), and of all of it 2 s and 8 s on (`-gateN-2s`,
/// `-gateN-8s`), with the pieces still near where they started and what
/// they touch at 2 s; the last time, with the other models mostly in its
/// box taken down and the island's statics in it (its ground aside) gone,
/// the same at 2 s and 8 s (`-gateN-alone-2s`, `-gateN-alone-8s`).
fn gates_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    use m2_math::glam::Vec3;
    let tick = TICK.as_secs_f32();
    let stem = out
        .file_stem()
        .map_or("shot".into(), |s| s.to_string_lossy());
    let picture = |name: String| out.with_file_name(format!("{stem}-{name}.ppm"));
    let Some(world) = &game.scene.world else {
        return Ok(());
    };
    let mut gates: Vec<(u32, Vec3)> = world
        .index
        .state_objects
        .iter()
        .filter(|(_, o)| o.model == GATE_MODEL)
        .map(|(&e, o)| (e, o.position))
        .collect();
    gates.sort_by_key(|g| g.0);
    eprintln!(
        "gates: {} placed: {}",
        gates.len(),
        gates
            .iter()
            .map(|(e, at)| format!("{e:#x} at {at:.1}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    game.on_foot = false;
    // Steps the game `seconds` on, drawing every tenth tick so the island
    // streams around the camera.
    let run = |backend: &mut WgpuBackend, game: &mut Game, seconds: f32| {
        for i in 0..(seconds / tick).round() as u32 {
            game.step(Intent::default(), tick);
            if i % 10 == 0 {
                render(backend, game, fov, aspect, None)?;
            }
        }
        anyhow::Ok(())
    };
    for (k, &(gate, position)) in gates.iter().enumerate() {
        let n = k + 1;
        // From the south, 50 m off and above the ground there, as the
        // mission comes at it.
        let look = |game: &mut Game, at: Vec3| {
            let mut eye = at + Vec3::new(0.0, 6.0, -50.0);
            if let Some(ground) = game
                .scene
                .world
                .as_ref()
                .and_then(|w| w.physics.island_below(eye.with_y(eye.y + 200.0), 400.0))
            {
                eye.y = eye.y.max(ground + 2.0);
            }
            game.camera = FlyCamera::looking_at(eye, at);
            eye
        };
        look(game, position + Vec3::Y * 8.0);
        game.scene.show_props([gate]);
        let started = Instant::now();
        // The gate, the start-up layers' models round it and the island
        // there all in.
        while game.scene.props.get(gate).is_none_or(|(_, loaded)| !loaded)
            || game.startup.as_ref().is_some_and(startup::Startup::loading)
            || game.scene.props.counts().loading + game.scene.props.counts().waiting_near > 0
            || game
                .scene
                .world
                .as_ref()
                .is_some_and(|w| w.stats().pending > 0)
        {
            if started.elapsed() > Duration::from_secs(120) {
                break;
            }
            run(backend, game, 0.5)?;
        }
        let (Some(centre), Some(top)) = (game.scene.props.centre(gate), game.scene.props.top(gate))
        else {
            eprintln!("gate {n} ({gate:#x}): its model never loaded");
            continue;
        };
        let eye = look(game, centre);
        let facing = game
            .scene
            .props
            .get(gate)
            .map_or(Vec3::Z, |(p, _)| p.rotation * Vec3::Z);
        eprintln!(
            "gate {n} ({gate:#x}) at {position:.1}, seen from {eye:.1}, its z axis {facing:.2}"
        );
        if let Some(world) = &game.scene.world {
            let props = &game.scene.props;
            let others = props.others_in_box(&world.physics, gate);
            eprintln!(
                "gate {n}: {} other models in its box, biggest first: {}",
                others.len(),
                others
                    .iter()
                    .take(GATE_LISTED)
                    .map(|(_, _, line)| line.as_str())
                    .collect::<Vec<_>>()
                    .join("; ")
            );
            if let Some((place, bounds)) = props.placed_bounds(gate) {
                let statics: Vec<String> = world
                    .physics
                    .statics_in_box(place, bounds)
                    .into_iter()
                    .filter(|&k| k != m2_engine::StaticKey::Entity(gate))
                    .map(|k| gate_static_name(world, props, k))
                    .collect();
                eprintln!(
                    "gate {n}: {} statics in its box: {}",
                    statics.len(),
                    statics.join("; ")
                );
            }
        }
        game.scene.props.skip = Some(gate);
        render(backend, game, fov, aspect, None)?;
        write_ppm(backend, &picture(format!("gate{n}-without")))?;
        game.scene.props.skip = None;
        // The bomb goes off on its top.
        let blast = top - Vec3::Y * 0.2;
        for (t, &(scale, alone)) in GATE_TRIES.iter().enumerate() {
            if alone && let Some(world) = &mut game.scene.world {
                // Everything else mostly in its box taken down, and the
                // island's statics in it but its ground.
                let props = &mut game.scene.props;
                let others: Vec<u32> = props
                    .others_in_box(&world.physics, gate)
                    .into_iter()
                    .filter(|o| o.1 >= GATE_ALONE_SHARE)
                    .map(|o| o.0)
                    .collect();
                let statics: Vec<m2_engine::StaticKey> = props
                    .placed_bounds(gate)
                    .map(|(place, bounds)| world.physics.statics_in_box(place, bounds))
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|k| {
                        matches!(
                            k,
                            m2_engine::StaticKey::Object(_) | m2_engine::StaticKey::Member(..)
                        )
                    })
                    .collect();
                props.hide(&mut world.physics, others.iter().copied());
                for &k in &statics {
                    world.physics.remove_static(k);
                }
                eprintln!(
                    "gate {n}, alone: took down {} other models and {} island statics",
                    others.len(),
                    statics.len()
                );
            }
            // Whole again.
            if let Some(world) = &mut game.scene.world {
                game.scene.props.hide(&mut world.physics, [gate]);
            }
            game.scene.show_props([gate]);
            run(backend, game, 0.1)?;
            let Some(world) = &mut game.scene.world else {
                break;
            };
            if !game
                .scene
                .props
                .wreck_scaled(&mut world.physics, gate, Some(blast), scale)
            {
                eprintln!("gate {n}: it would not break");
                break;
            }
            let try_n = t + 1;
            eprintln!(
                "gate {n}, try {try_n} ({scale} times as fast{}), blast at {blast:.1}: sets off: \
                 {}",
                if alone { ", alone" } else { "" },
                game.scene
                    .props
                    .pieces_report(&world.physics, gate)
                    .unwrap_or_else(|| "no pieces".to_owned())
            );
            let first = t == 0;
            let tag = if alone { "-alone" } else { "" };
            if first {
                // Burning where its break-up says, as in play.
                for at in game.scene.props.breakup_points(gate) {
                    game.combat.blaze(&world.physics, at);
                }
                for (parts, name) in [
                    (props::Parts::Rest, "rest"),
                    (props::Parts::Pieces, "pieces"),
                ] {
                    game.scene.props.parts = parts;
                    render(backend, game, fov, aspect, None)?;
                    write_ppm(backend, &picture(format!("gate{n}-{name}")))?;
                }
                game.scene.props.parts = props::Parts::All;
            }
            let mut gone = 0.0;
            for (on, name) in [(2.0, "2s"), (8.0, "8s")] {
                run(backend, game, on - gone)?;
                gone = on;
                let Some(world) = &game.scene.world else {
                    break;
                };
                let props = &game.scene.props;
                eprintln!(
                    "gate {n}, try {try_n}: {on} s on: {}; {}",
                    props.pieces_moved(&world.physics, gate).unwrap_or_default(),
                    props.describe(&world.physics, gate).unwrap_or_default()
                );
                if (first || alone) && on < 8.0 {
                    let held: Vec<String> = props
                        .pieces_held(&world.physics, gate, GATE_HELD)
                        .into_iter()
                        .map(|(node, keys)| {
                            let by: Vec<String> = keys
                                .into_iter()
                                .map(|k| gate_static_name(world, props, k))
                                .collect();
                            format!(
                                "n{node} by {}",
                                if by.is_empty() {
                                    "nothing static".to_owned()
                                } else {
                                    by.join(", ")
                                }
                            )
                        })
                        .collect();
                    eprintln!(
                        "gate {n}, try {try_n}: {on} s on, {} pieces within {GATE_HELD} m of \
                         where they started: {}",
                        held.len(),
                        held.join("; ")
                    );
                }
                if first || alone {
                    render(backend, game, fov, aspect, None)?;
                    write_ppm(backend, &picture(format!("gate{n}{tag}-{name}")))?;
                }
            }
        }
    }
    Ok(())
}

/// Draws a frame with the HUD (and the mission's objectives) and writes
/// it to `out` as a binary PPM.
fn hud_picture(
    backend: &mut WgpuBackend,
    game: &mut Game,
    link: &mission::MissionLink,
    fov: f32,
    aspect: f32,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let (w, h) = backend.size();
    let mut hud = ui::Hud::new();
    let mut settings = config::Config::default();
    let info = hud_info(game, Some(link), fov, aspect);
    let _ = hud.build(ui::headless_input(w, h, 1.0), &info, &mut settings);
    render(backend, game, fov, aspect, Some(&mut hud))?;
    write_ppm(backend, out)
}

/// Writes the frame just rendered offscreen as a binary PPM.
fn write_ppm(backend: &WgpuBackend, out: &std::path::Path) -> anyhow::Result<()> {
    let (w, h) = backend.size();
    let rgba = backend.read_pixels()?;
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    ppm.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(out, ppm).with_context(|| format!("writing {}", out.display()))
}

/// A picture of a delivery, from 30 m off its drop point `to` on her
/// side and 6 m up (clear of the ground there), looking at `at`; written
/// to `out`, the camera going back to her after.
#[allow(clippy::too_many_arguments)]
fn delivery_picture(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    to: m2_math::glam::Vec3,
    at: m2_math::glam::Vec3,
    eye: Option<m2_math::glam::Vec3>,
    out: &std::path::Path,
) -> anyhow::Result<m2_math::glam::Vec3> {
    use m2_math::glam::Vec3;
    let eye = eye.unwrap_or_else(|| {
        let away = (game.focus() - to)
            .with_y(0.0)
            .try_normalize()
            .unwrap_or(Vec3::X);
        let mut eye = to + away * 30.0 + Vec3::Y * 6.0;
        if let Some(world) = &game.scene.world {
            let top = eye + Vec3::Y * 300.0;
            if let Some((t, _)) = world.physics.cast_ray(top, Vec3::NEG_Y, 600.0) {
                eye.y = eye.y.max(top.y - t + 2.0);
            }
        }
        eye
    });
    let (on_foot, camera) = (game.on_foot, game.camera);
    game.on_foot = false;
    game.camera = FlyCamera::looking_at(eye, at);
    render(backend, game, fov, aspect, None)?;
    let written = write_ppm(backend, out);
    game.on_foot = on_foot;
    game.camera = camera;
    written?;
    eprintln!(
        "delivery picture: wrote {} from {eye:.1} looking at {at:.1}",
        out.display()
    );
    Ok(eye)
}

/// How she holds her weapon, for close-ups: where its barrel and its top
/// point (ahead, to her right, up; and degrees below level), and where the
/// grip and muzzle are from her right hand's attach point, in metres
/// (ahead, right, up).
fn describe_hold(game: &Game) -> Option<String> {
    use m2_math::glam::Vec3;
    let p = game.scene.player.as_ref()?;
    let (_, rig) = p.model.as_ref()?;
    let arsenal = &game.scene.arsenal;
    let w = match game.combat.weapon {
        combat::Weapon::Rifle => arsenal.rifle.as_ref()?,
        combat::Weapon::Rpg => arsenal.rpg.as_ref()?,
        combat::Weapon::GrenadeLauncher => arsenal.launcher.as_ref()?,
    };
    let (nodes, _) = rig.posed();
    let body = p.model_matrix();
    let held = arsenal.in_hand(
        w,
        body,
        &nodes,
        (arsenal.hand, arsenal.attach),
        rig.two_handed,
    )?;
    let ahead = Vec3::new(p.facing.sin(), 0.0, p.facing.cos());
    let right = Vec3::new(-ahead.z, 0.0, ahead.x);
    let local = |v: Vec3| Vec3::new(v.dot(ahead), v.dot(right), v.y);
    let barrel = held.transform_vector3(w.muzzle).normalize_or_zero();
    let top = held.transform_vector3(Vec3::Y).normalize_or_zero();
    let palm = (body * nodes[arsenal.attach?]).w_axis.truncate();
    let grip = held.w_axis.truncate();
    let muzzle = held.transform_point3(w.muzzle);
    Some(format!(
        "{:?} held {}, barrel {:.2} ({:.0} deg below level), top {:.2}, grip {:.2} and muzzle {:.2} from the palm",
        game.combat.weapon,
        if rig.two_handed {
            "two-handed"
        } else {
            "in the right hand"
        },
        local(barrel),
        (-barrel.y).asin().to_degrees(),
        local(top),
        local(grip - palm),
        local(muzzle - palm),
    ))
}

/// How the weapons on her back hang: for each, which side, which way its
/// barrel and its top point (ahead, right, up of her), and where its grip
/// and muzzle are from the middle of her back at the attach points'
/// height.
fn describe_slung(game: &Game) -> Vec<String> {
    use m2_math::glam::Vec3;
    let Some(p) = game.scene.player.as_ref() else {
        return Vec::new();
    };
    let Some((_, rig)) = p.model.as_ref() else {
        return Vec::new();
    };
    let arsenal = &game.scene.arsenal;
    let (nodes, _) = rig.posed();
    let body = p.model_matrix();
    let ahead = Vec3::new(p.facing.sin(), 0.0, p.facing.cos());
    let right = Vec3::new(-ahead.z, 0.0, ahead.x);
    let local = |v: Vec3| Vec3::new(v.dot(ahead), v.dot(right), v.y);
    let holding = !game.combat.designator && p.throwing.is_none() && !p.down;
    let mut lines = Vec::new();
    for (slot, slung) in game.combat.slung(holding).into_iter().enumerate() {
        let Some(weapon) = slung else { continue };
        let model = match weapon {
            combat::Weapon::Rifle => arsenal.of_gun(&game.combat.rifle),
            combat::Weapon::Rpg | combat::Weapon::GrenadeLauncher => {
                arsenal.of_gun(&game.combat.launcher)
            }
        };
        let side = ["right", "left"][slot];
        let Some((w, at)) = model.and_then(|w| Some((w, arsenal.on_back(w, body, &nodes, slot)?)))
        else {
            lines.push(format!(
                "{weapon:?} on her {side}: no model or attach point"
            ));
            continue;
        };
        let middle = body.transform_point3(Vec3::new(0.0, 1.4, 0.0));
        // Along its length (its box's longest side), toward its muzzle's end.
        let (lo, hi) = w.model.bounds;
        let size = hi - lo;
        let long = (0..3).fold(0, |a, b| if size[b] > size[a] { b } else { a });
        let mut length = Vec3::ZERO;
        length[long] = if w.muzzle[long] < (lo[long] + hi[long]) * 0.5 {
            -1.0
        } else {
            1.0
        };
        let barrel = at.transform_vector3(length).normalize_or_zero();
        let top = at.transform_vector3(Vec3::Y).normalize_or_zero();
        // Its box's corners, from her middle (ahead, right, up).
        let corners: Vec<Vec3> = (0..8)
            .map(|k| {
                let pick = |bit: usize, a: f32, b: f32| if k & bit == 0 { a } else { b };
                let c = Vec3::new(
                    pick(1, lo.x, hi.x),
                    pick(2, lo.y, hi.y),
                    pick(4, lo.z, hi.z),
                );
                local(at.transform_point3(c) - middle)
            })
            .collect();
        let span = |i: usize| {
            corners
                .iter()
                .map(|c| c[i])
                .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v), b.max(v)))
        };
        let (ahead_span, right_span, up_span) = (span(0), span(1), span(2));
        let muzzle_end = if barrel.y < 0.0 { "down" } else { "up" };
        lines.push(format!(
            "{weapon:?} on her {side}: length {:.2} ({:.0} deg above level, muzzle end {muzzle_end}), \
             from {:.2} to {:.2} m above her feet, {:.2} to {:.2} m to her right, {:.2} to {:.2} m \
             ahead of her middle; top {:.2}, grip {:.2} and muzzle {:.2} from her back's middle",
            local(barrel),
            barrel.y.asin().to_degrees(),
            up_span.0 + 1.4,
            up_span.1 + 1.4,
            right_span.0,
            right_span.1,
            ahead_span.0,
            ahead_span.1,
            local(top),
            local(at.w_axis.truncate() - middle),
            local(at.transform_point3(w.muzzle) - middle),
        ));
    }
    lines
}

/// Puts the camera `reach` metres from the mission marker nearest the
/// player, on her side of it and a little above, looking at it, and lets
/// the island stream in around it.
fn look_at_marker(
    backend: &mut WgpuBackend,
    game: &mut Game,
    link: &mission::MissionLink,
    reach: f32,
    fov: f32,
    aspect: f32,
) -> anyhow::Result<()> {
    use m2_math::glam::Vec3;
    let here = game.focus();
    let Some((at, label)) = link
        .view()
        .markers
        .into_iter()
        .min_by(|a, b| a.at.distance(here).total_cmp(&b.at.distance(here)))
        .map(|m| (m.at, m.label()))
    else {
        eprintln!("mission look: no markers");
        return Ok(());
    };
    let side = (here - at).with_y(0.0).normalize_or(Vec3::X);
    let eye = at + side * reach + Vec3::Y * (reach * 0.4);
    eprintln!("mission look: {label} at {at:.1} from {eye:.1}");
    game.on_foot = false;
    game.camera = FlyCamera::looking_at(eye, at);
    let tick = TICK.as_secs_f32();
    let started = Instant::now();
    for i in 0.. {
        game.step(Intent::default(), tick);
        if i % 10 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
        let pending = game.scene.world.as_ref().map_or(0, |w| w.stats().pending);
        if (i > 120 && pending == 0) || started.elapsed() > Duration::from_secs(60) {
            break;
        }
    }
    Ok(())
}

/// [`static_name`], with the layer that put up a state layer's model
/// (the gate test's lists).
fn gate_static_name(
    world: &m2_engine::World,
    props: &props::Props,
    key: m2_engine::StaticKey,
) -> String {
    let name = static_name(world, Some(key));
    match key {
        m2_engine::StaticKey::Entity(e) => {
            format!("{name} of {}", props.layer(e).unwrap_or("no layer noted"))
        }
        _ => name,
    }
}

/// What a static collider is: a placed object and its model, a building
/// of a batch, a terrain cell or a state layer's model (`None`: something
/// not static, such as a vehicle).
pub(crate) fn static_name(world: &m2_engine::World, key: Option<m2_engine::StaticKey>) -> String {
    use m2_engine::StaticKey;
    match key {
        Some(StaticKey::Object(i)) => world.index.objects.get(i).map_or_else(
            || format!("object {i}"),
            |o| format!("object {i}, model {:08x} at {:.1}", o.model, o.position),
        ),
        Some(StaticKey::Member(i, k)) => world.index.objects.get(i).map_or_else(
            || format!("building {k} of object {i}"),
            |o| format!("building {k} of object {i} (batch {:08x})", o.model),
        ),
        Some(StaticKey::Cell(h)) => format!("terrain cell {h:08x}"),
        Some(StaticKey::Entity(e)) => world.index.state_objects.get(&e).map_or_else(
            || format!("state layer entity {e:08x}"),
            |o| format!("state layer entity {e:08x}, model {:08x}", o.model),
        ),
        None => "something not static".to_owned(),
    }
}

/// Looks for what stops vehicles where she stands: rays out to 25 m in
/// 16 directions at four heights over the ground, and the ground's height
/// every metre along lines through the spot, printing what each hits.
fn probe_for_screenshot(game: &Game) {
    use m2_engine::StaticKey;
    use m2_math::glam::Vec3;
    let (Some(world), Some(player)) = (&game.scene.world, &game.scene.player) else {
        return;
    };
    let here = player.character.position;
    let what = |key: Option<StaticKey>| static_name(world, key);
    let down = |x: f32, z: f32| {
        let top = here.y + 60.0;
        world
            .physics
            .cast_ray_keyed(Vec3::new(x, top, z), -Vec3::Y, 200.0)
            .map(|(d, n, k)| (top - d, n, k))
    };
    let Some((ground, normal, key)) = down(here.x, here.z) else {
        eprintln!("probe: no ground under {here:.1}");
        return;
    };
    eprintln!(
        "probe: at {here:.1}, ground {ground:.2}, normal {normal:.2} ({})",
        what(key)
    );
    for up in [0.2, 0.6, 1.2, 2.5] {
        let from = Vec3::new(here.x, ground + up, here.z);
        let mut hits = Vec::new();
        for k in 0..16 {
            let a = k as f32 * std::f32::consts::TAU / 16.0;
            let dir = Vec3::new(a.sin(), 0.0, a.cos());
            if let Some((d, n, key)) = world.physics.cast_ray_keyed(from, dir, 25.0) {
                hits.push(format!(
                    "{:>3.0} deg: {d:4.1} m, normal {n:.2}, {}",
                    a.to_degrees(),
                    what(key)
                ));
            }
        }
        eprintln!("probe: {up} m up, {} of 16 rays hit:", hits.len());
        for h in hits {
            eprintln!("  {h}");
        }
    }
    for (label, axis) in [("x", Vec3::X), ("z", Vec3::Z)] {
        let mut last: Option<(f32, Option<StaticKey>)> = None;
        let mut notes = Vec::new();
        for step in -30..=30 {
            let p = here + axis * step as f32;
            match down(p.x, p.z) {
                Some((y, _, key)) => {
                    if let Some((ly, lk)) = last {
                        if (y - ly).abs() > 0.25 {
                            notes.push(format!("{:+.2} m at {step:+} ({})", y - ly, what(key)));
                        } else if lk != key {
                            notes.push(format!("onto {} at {step:+}", what(key)));
                        }
                    }
                    last = Some((y, key));
                }
                None => notes.push(format!("no ground at {step:+}")),
            }
        }
        eprintln!(
            "probe: along {label} from -30 to +30 m: {}",
            if notes.is_empty() {
                "smooth".to_owned()
            } else {
                notes.join("; ")
            }
        );
    }
}

/// Starts the mission scripts, waits for them to read the archive and
/// start the first contract, then plays on while the island streams in
/// where they put the player, printing the objectives.
/// Offscreen, the front end alone (`--front-end`): the shell's own menus
/// start as after the boot movies, and once up are pictured
/// (`<out>-front-<n>.ppm`) before the first of `keys` (Flash's key
/// codes), and a second and a half after each; their backdrop movie plays
/// in its box under them. Ends two seconds after the last key, when they
/// start a new game, or after a minute without them coming up.
fn front_end_offscreen(
    config: &Config,
    install: &Install,
    (keys, clicks): (&[i32], &[(usize, [f32; 2])]),
    out: &std::path::Path,
) -> anyhow::Result<Option<PathBuf>> {
    let mut config = config.clone();
    let (w, h) = (config.window.width, config.window.height);
    let mut backend = WgpuBackend::headless(w, h)?;
    let mut game = build_scene(
        &mut backend,
        None,
        &Mode::Models(Showcase {
            models: Vec::new(),
            gait: Gait::Idle,
            clips: Vec::new(),
            turn: 0.0,
        }),
        false,
        config.world.sea_level,
        false,
        config.hero,
    )?;
    let mut hud = ui::Hud::new();
    let mut shell = gamehud::GameHud::start_shell(install);
    let modes = display::primary_modes();
    let ffmpeg = movie::ffmpeg();
    let mut backdrop: Option<(movie::Movie, [f32; 4])> = None;
    let mut texture: Option<egui::TextureHandle> = None;
    let stem = out
        .file_stem()
        .map_or("shot".into(), |s| s.to_string_lossy().into_owned());
    let step = 1.0 / 30.0;
    let (mut t, mut up_at) = (0.0f64, None::<f64>);
    let mut pictured = 0usize;
    // The player's settings and saves, as the menus show them, with the
    // video screen as for a window of the picture's size, its lists the
    // primary monitor's modes.
    let settings = pause::Settings {
        profile: config.hero.full_name().into(),
        video: pause::Video {
            width: w,
            height: h,
            fullscreen: None,
        },
        modes: modes.clone(),
        saves: pause_saves(config.hero),
        ..config.menu_settings()
    };
    eprintln!("front end: {} saves listed", settings.saves.len());
    let mut chosen = None;
    // `M2_SHELL_PRESS_START`: start the shell from its press-start
    // screen first, the keys counting from then.
    let mut waiting_start = std::env::var_os("M2_SHELL_PRESS_START").is_some();
    let (mut attract_seen, mut woken) = (false, false);
    loop {
        t += step;
        let since = up_at.map(|u| t - u);
        // Keys at 3 s, 5 s...; pictures 1.5 s before each and after the
        // last.
        let mut pressed = Vec::new();
        // Up, the menus wait at their press-start screen: Enter each
        // second until it takes, and the keys count from then (the legal
        // screen before it takes a varying while).
        if waiting_start && shell.ready() {
            if shell.shell().started {
                waiting_start = false;
                eprintln!("front end: started from press-start at {t:.1} s");
                up_at = Some(t);
            } else if (t / 1.0).fract() < step {
                pressed.extend([(13, true), (13, false)]);
            }
        }
        let mut picture = false;
        let mut pointer = None;
        let mut clicked = Vec::new();
        if let Some(since) = since {
            for (i, &k) in keys.iter().enumerate() {
                let at = 3.0 + 2.0 * i as f64;
                let up = at + 0.2;
                let (down_now, up_now) = (
                    since < at && since + step >= at,
                    since < up && since + step >= up,
                );
                if k == CLICK {
                    // The pointer goes there a frame before the button.
                    let Some(&(_, p)) = clicks.iter().find(|(n, _)| *n == i) else {
                        continue;
                    };
                    if since + 2.0 * step >= at && since < up + step {
                        pointer = Some(p);
                    }
                    if down_now || up_now {
                        clicked.push(down_now);
                        if down_now {
                            eprintln!("front end: click at {p:?} at {t:.1} s");
                        }
                    }
                    continue;
                }
                // Key 0 only waits.
                if down_now && k != 0 {
                    pressed.push((k, true));
                    eprintln!("front end: key {k} at {t:.1} s");
                }
                // Let go a fifth of a second on, as a hand does.
                if up_now && k != 0 {
                    pressed.push((k, false));
                }
            }
            for i in 0..=keys.len() {
                let at = 1.5 + 2.0 * i as f64;
                if since < at && since + step >= at {
                    picture = true;
                }
            }
            if since > 4.0 + 2.0 * keys.len() as f64 {
                break;
            }
        } else if t > if waiting_start { 180.0 } else { 60.0 } {
            eprintln!("front end: the menus did not come up in a minute");
            break;
        }
        if shell.failed() {
            break;
        }
        // The backdrop the menus ask for.
        let wanted = shell.shell().movie.clone();
        if wanted.as_ref().map(|(n, _)| n) != backdrop.as_ref().map(|(m, _)| &m.name) {
            backdrop = None;
            if let (Some((name, rect)), Some(ffmpeg)) = (&wanted, &ffmpeg) {
                eprintln!("front end: plays {name} in {rect:?}");
                match movie::Movie::open(ffmpeg, install, name) {
                    Ok(m) => backdrop = Some((m, *rect)),
                    Err(e) => eprintln!("front end: {name}: {e:#}"),
                }
            }
        }
        if let Some((m, rect)) = &mut backdrop {
            *rect = wanted.map_or(*rect, |(_, r)| r);
            // Whole frames over its rate, looping at its end.
            let due = (t * m.info.fps.max(1.0)) as u64;
            while m.frame.as_ref().is_none_or(|(n, _)| *n < due) {
                if m.next_frame().is_none() {
                    break;
                }
            }
        }
        // Left long enough, the menus wait in their attract state (see
        // `shell::ATTRACT_STATE`): the next key (not 0) or click takes them
        // back, and they don't see it.
        let attract = shell.shell().attract;
        if attract != attract_seen {
            attract_seen = attract;
            woken = false;
            eprintln!(
                "front end: {} at {t:.1} s",
                if attract {
                    "the menus wait in their attract state"
                } else {
                    "the menus are back"
                }
            );
        }
        if attract && (pressed.iter().any(|&(_, down)| down) || clicked.contains(&true)) {
            woken = true;
            pressed.clear();
            clicked.clear();
        }
        let input = ui::headless_input(w, h, t);
        let state = gamehud::HudState {
            size: [w as f32, h as f32],
            dt: if up_at.is_some() || (waiting_start && shell.ready()) {
                step
            } else {
                0.0
            },
            shell_wake: woken,
            keys: pressed,
            pause_settings: settings.clone(),
            pointer,
            clicks: clicked,
            ..Default::default()
        };
        let _ = hud.build_screen(input, |ui| {
            let ctx = ui.ctx().clone();
            shell.update_and_wait(&ctx, state.clone(), Duration::from_secs(5));
            let screen = ctx.content_rect();
            let painter = ui.painter_at(screen);
            painter.rect_filled(screen, 0.0, egui::Color32::BLACK);
            // `M2_BACKDROP_TOP`: the backdrop over the menus, to see what
            // covers it.
            let top = std::env::var_os("M2_BACKDROP_TOP").is_some();
            if top {
                shell.paint(&painter);
            } else {
                shell.paint_under(&painter);
            }
            if let Some((m, [x0, y0, x1, y1])) = &backdrop
                && let Some((_, rgba)) = &m.frame
                && picture
            {
                let size = [m.info.width as usize, m.info.height as usize];
                let image = egui::ColorImage::from_rgba_unmultiplied(size, rgba);
                let t = texture.get_or_insert_with(|| {
                    ctx.load_texture("backdrop", image.clone(), egui::TextureOptions::LINEAR)
                });
                t.set(image, egui::TextureOptions::LINEAR);
                painter.image(
                    t.id(),
                    egui::Rect::from_min_max(egui::pos2(*x0, *y0), egui::pos2(*x1, *y1)),
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            if !top {
                shell.paint_over(&painter);
            }
        });
        if up_at.is_none() && shell.ready() && !waiting_start {
            eprintln!("front end: the menus are up after {t:.1} s");
            up_at = Some(t);
        }
        let news = shell.take_shell_news();
        if let Some(v) = news.video {
            eprintln!(
                "front end: video kept: {}, {} (a window would now be {})",
                v.resolution_text(),
                v.refresh_text(),
                if v.fullscreen.is_some() {
                    "fullscreen at that mode"
                } else {
                    "resized to it"
                }
            );
        }
        for unused in &news.unused {
            eprintln!("front end: {unused} kept in the menu; the alpha has nothing for it");
        }
        if let Some(s) = &news.settings {
            config.keep_menu_settings(s);
            config.save("front end");
        }
        if let Some(table) = news.keys {
            table.apply(&mut config.keys);
            config.save("front end");
        }
        if let Some(n) = news.delete {
            let path = pause_save_path(n);
            match std::fs::remove_file(&path) {
                Ok(()) => eprintln!("front end: save {n} deleted ({})", path.display()),
                Err(e) => eprintln!("front end: could not delete {}: {e}", path.display()),
            }
        }
        let load = if news.continue_game {
            newest_save()
        } else {
            news.load
        };
        if let Some(n) = load {
            eprintln!(
                "front end: {} slot {n}",
                if news.continue_game {
                    "Continue:"
                } else {
                    "Load Game:"
                }
            );
            chosen = Some(pause_save_path(n));
        }
        if news.quit {
            eprintln!("front end: Exit");
        }
        if let Some(who) = shell.take_new_game() {
            eprintln!(
                "front end: a new game as {who} ({})",
                shell::hero(&who).map_or("no hero", Hero::name)
            );
            picture = true;
        }
        if picture {
            if let Some((m, rect)) = &backdrop {
                eprintln!(
                    "front end: backdrop {} frame {:?} in {rect:?}, over {} prims",
                    m.name,
                    m.frame.as_ref().map(|(n, _)| n),
                    shell.shell().under
                );
                // The backdrop's frame alone, to tell it from what covers
                // it.
                if let Some((_, rgba)) = &m.frame {
                    let (bw, bh) = (m.info.width, m.info.height);
                    let mut ppm = format!("P6\n{bw} {bh}\n255\n").into_bytes();
                    ppm.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
                    let path = out.with_file_name(format!("{stem}-backdrop-{pictured}.ppm"));
                    let _ = std::fs::write(path, ppm);
                }
            }
            render(
                &mut backend,
                &mut game,
                60.0,
                w as f32 / h as f32,
                Some(&mut hud),
            )?;
            let path = out.with_file_name(format!("{stem}-front-{pictured}.ppm"));
            write_ppm(&backend, &path)?;
            eprintln!("front end: pictured {}", path.display());
            pictured += 1;
        }
        if chosen.is_some() {
            break;
        }
    }
    eprintln!("front end: done after {t:.1} s");
    Ok(chosen)
}

/// Offscreen, the movies the scripts are waiting on (`--movies`): each is
/// opened, a frame three seconds in is pictured (`<out>-movie-<name>.ppm`)
/// and its sound decoded, and the scripts are told it is over.
fn movies_offscreen(link: &mut mission::MissionLink, install: &Install, out: &std::path::Path) {
    while let Some((name, lines)) = link.take_movie() {
        eprintln!("movies: the scripts play {name}");
        let captions = lines.as_ref().map(|lines| {
            let strings = link.strings().cloned().unwrap_or_default();
            (subtitles::Captions::load(install, strings), lines.clone())
        });
        match &captions {
            None => eprintln!("movies: {name} has no subtitles"),
            Some((Err(e), _)) => eprintln!("movies: {name} subtitles: {e}"),
            Some((Ok(c), lines)) => {
                for l in lines {
                    eprintln!(
                        "movies: {name} subtitle at {:.2} s{}{}: {}",
                        l.time,
                        l.seconds
                            .map(|s| format!(" for {s:.2} s"))
                            .unwrap_or_default(),
                        if l.top { " (top)" } else { "" },
                        c.plain(&l.text).replace('\n', " / ")
                    );
                }
            }
        }
        match movie::ffmpeg() {
            None => eprintln!("movies: no ffmpeg; {name} skipped"),
            Some(ffmpeg) => match movie::Movie::open(&ffmpeg, install, &name) {
                Err(e) => eprintln!("movies: {name}: {e:#}"),
                Ok(mut m) => {
                    // Three seconds in, or half a second into its first
                    // subtitle, with the subtitles up then drawn on it.
                    let mut shown = captions.as_ref().and_then(|(c, lines)| {
                        let c = c.as_ref().ok()?;
                        Some((c, subtitles::MovieSubtitles::new(lines.clone(), true)?))
                    });
                    let first = captions.as_ref().and_then(|(_, lines)| {
                        lines
                            .iter()
                            .filter(|l| !l.top && l.time > 0.0)
                            .map(|l| l.time)
                            .reduce(f64::min)
                    });
                    let seconds = first.map_or(3.0, |t| t + 0.5);
                    // The scripts count the movie's time as its frame over
                    // 30.
                    let at = if first.is_some() {
                        (seconds * 30.0) as usize
                    } else {
                        (m.info.fps * seconds) as usize
                    };
                    let (w, h) = (m.info.width, m.info.height);
                    let mut frame = None;
                    for n in 0..=at {
                        match m.next_frame() {
                            Some(f) => frame = Some(f.to_vec()),
                            None => break,
                        }
                        if let Some((_, s)) = &mut shown {
                            s.update(n as u64);
                        }
                    }
                    if let (Some(rgba), Some((c, s))) = (&mut frame, &shown) {
                        c.composite(rgba, w, h, s.bottom(), s.top());
                    }
                    let stem = out
                        .file_stem()
                        .map_or("shot".into(), |s| s.to_string_lossy());
                    let path = out.with_file_name(format!("{stem}-movie-{name}.ppm"));
                    if let Some(rgba) = frame {
                        let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
                        ppm.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
                        match std::fs::write(&path, ppm) {
                            Ok(()) => eprintln!("movies: pictured {name}: {}", path.display()),
                            Err(e) => eprintln!("movies: {}: {e}", path.display()),
                        }
                    }
                    eprintln!("movies: {name} sound {:?} s", m.sound_seconds());
                }
            },
        }
        link.movie_done();
    }
}

fn mission_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    install: &Install,
    options: &Options,
    fov: f32,
    aspect: f32,
    out: &std::path::Path,
) -> anyhow::Result<mission::MissionLink> {
    let tick = TICK.as_secs_f32();
    let mut link = mission::MissionLink::start(install, options.mission_drivers.clone())
        .with_save(options.load.clone())
        .with_support(options.support.clone())
        .with_movies(options.movies)
        .with_skip_mode_off(options.skip_mode_off)
        .with_skip_to(options.skip_to.clone())
        .with_job_near(options.job_near);
    let started = Instant::now();
    while !link.running() && started.elapsed() < Duration::from_secs(300) {
        link.tick(game, tick);
        game.dress(backend);
        if options.movies {
            movies_offscreen(&mut link, install, out);
        }
        if link
            .view()
            .status
            .is_some_and(|s| s.starts_with("Mission scripts stopped"))
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!(
        "mission scripts {} after {:.1} s",
        if link.running() {
            "running"
        } else {
            "not running"
        },
        started.elapsed().as_secs_f32()
    );
    // Ten seconds of play (or as asked), streaming as it goes; with
    // --autoplay she plays it herself, until the contract is done.
    let seconds = if options.mission_seconds > 0.0 {
        options.mission_seconds
    } else {
        10.0
    };
    let mut auto = options.autoplay.then(autoplay::Autoplay::default);
    // How long the scripts' transit map has been up.
    let mut transit_up = 0.0f32;
    let mut contract_done: Option<u32> = None;
    // With --contracts N, she plays on through the later contracts: the
    // tick the Nth was done, and how many have been.
    let mut play_done: Option<u32> = None;
    let mut contracts_seen = 0;
    let mut late_hud: Option<(ui::Hud, Config)> = None;
    // The loading screen's steps over the scripts' fades until the late HUD
    // (which draws it) runs, logged.
    let mut screen_cycle = loading::Cycle::default();
    let played = Instant::now();
    let every = if auto.is_some() { 10 } else { 30 };
    // With --autoplay, a picture of each delivery as its helicopter
    // hovers over the drop point (`<out>-delivery1.ppm`, ...), and one
    // from the same place once the load has come down (`-down`).
    let stem = out
        .file_stem()
        .map_or("shot".into(), |s| s.to_string_lossy());
    let picture = |name: String| out.with_file_name(format!("{stem}-{name}.ppm"));
    let mut pictured: HashSet<u32> = HashSet::new();
    let mut down: Vec<(u32, usize, m2_math::glam::Vec3, m2_math::glam::Vec3)> = Vec::new();
    // And the start's boat from the chase camera once it planes (round
    // 89's, five seconds after she was seated, caught it barely moving),
    // or twenty seconds after she is seated if it never does (`-boat`),
    // and her three seconds after she gets out of it (`-ashore`): the
    // tick she was seated and the tick each picture is due.
    let mut seated: Option<u32> = None;
    let mut boat_shot: Option<u32> = None;
    let mut ashore_shot: Option<u32> = None;
    // And what each crate broken open left, as soon as it lies there
    // (she takes it within a second or two), from 5 m off (`-pickupN`):
    // the crates seen, and the pictures due (tick, crate).
    let mut opened: HashSet<(u32, u32)> = HashSet::new();
    // When to picture each, which, and its number.
    let mut pickup_shots: Vec<(u32, (u32, u32), usize)> = Vec::new();
    // And the satellite's view as the timing wheel comes up for each
    // call (`-satelliteN`), her PDA as she opens it (`-pdaN`), and her own
    // view two seconds after each bomb lands (`-bombN`) and eight
    // (`-bombN-late`, when the videos' gates lie broken: round 96), all
    // with the HUD: whether the wheel (the PDA) was up, the pictures taken
    // and those due (tick, number, seconds after).
    let mut wheel_up = false;
    let mut satellite_shots = 0;
    let (mut pda_shots, mut pda_up) = (0, false);
    let mut bombs = 0;
    let mut bomb_shots: Vec<(u32, usize, u32, m2_math::glam::Vec2)> = Vec::new();
    // The tutorial box's line as last shown, its keys named (round 99's
    // `[sprint]` showed in no picture).
    let mut tutorial_shown: Option<String> = None;
    // The clips' sound events met, by category and name: how many hers,
    // how many others' within earshot.
    let mut heard: std::collections::BTreeMap<(String, String), (u32, u32)> = Default::default();
    let mut strike_seen: Option<u32> = None;
    // With --hijack, the hijacks run as in the game, and she hijacks the
    // vehicle an objective's "action" marker is on (PmcCon001's villa
    // tank): once the marker has been up HIJACK_WAIT (the tank comes
    // through the wall first), she is put beside it ([`hijack::beside`]),
    // presses the use key a second later and answers its prompts at once
    // ([`hijack::Autopad`]). For testing.
    let mut hijacks = options
        .hijack
        .then(|| hijack::Hijacks::start(install, options.hijack_crew.clone()));
    let mut hijack_input = Input::default();
    let mut hijack_pad = hijack::Autopad::default();
    let (mut hijack_tries, mut hijack_use, mut hijack_t) = (0, None, 0.0);
    // The scripts' own hijacks ([`ridehijack`]), answered as --hijack's.
    let (mut ride_input, mut ride_pad, mut ride_t) =
        (Input::default(), hijack::Autopad::default(), 0.0f32);
    let (mut hijack_running, mut hijack_shots) = (0.0f32, 0u32);
    let mut action_since: Option<u32> = None;
    let mut placed_tried = std::collections::HashSet::new();
    // Seconds she has been where --props-check hits the props.
    let mut props_ready = 0.0;
    let mut shop_check = options.shop_check.clone().map(shopcheck::ShopCheck::new);
    let mut cheat_check = options.cheat_check.then(cheats::Check::default);
    let mut context = ContextCheck::default();
    let (mut dialog_up, mut dialog_pressed) = (0.0f32, 0usize);
    let (mut medevac_done, mut medevac_from) = (false, None::<(f32, m2_math::glam::Vec3)>);
    let mut visit: Option<f32> = None;
    // `--call-extraction`: its step (0 waiting, 1 menu up, 2 calling, 3 by
    // it), the designator's tap, and seconds the helicopter has been about.
    let (mut extraction, mut extraction_tap, mut extraction_seen) = (0u8, false, 0.0f32);
    let (mut locked_was, mut locked_for) = (false, 0.0f32);
    let mut on_contract: Option<(String, i64)> = None;
    // Message box calls with a callback, while the game's HUD isn't
    // running: when each is due back, by call number; the last seen.
    let (mut boxed, mut boxed_seen): (Vec<(f32, u64)>, u64) = (Vec::new(), 0);
    for i in 0..(seconds / tick).round() as u32 {
        let intent = match auto.as_mut().filter(|_| play_done.is_none()) {
            Some(a) => {
                let contract = link.contract();
                if contract != on_contract {
                    eprintln!(
                        "autoplay {:.1} s: on contract {:?}",
                        i as f32 * tick,
                        contract
                    );
                    on_contract = contract;
                }
                a.sights = link.sight_targets(game);
                let keep = link.kept_within();
                if keep.len() != a.keep.len() {
                    eprintln!(
                        "autoplay: kept within {} region(s) the scripts watch her leave",
                        keep.len()
                    );
                }
                a.keep = keep;
                let marks = link.contract_marks();
                a.offers = link.offers(game);
                let objectives = link.view().objectives;
                a.step(game, &marks, &objectives, &link.awaited(), tick)
            }
            None => Intent::default(),
        };
        if let Some(hp) = options.hurt
            && i == (HURT_AT / tick).round() as u32
        {
            eprintln!("hurt: her health {:.1} -> {hp:.1}", game.combat.hp);
            game.combat.hp = hp;
        }
        // With --support she throws the marker the scripts hand her.
        let mut intent = intent;
        if let Some(n) = options.honk_every.filter(|n| *n > 0.0) {
            intent.honk = (i as f32 * tick) % n < 0.5;
        }
        // With --jump-with-enter, the Enter pressed after the contract (to
        // close a fanfare) is the pad's A, which jumps too.
        if options.jump_with_enter
            && contract_done.is_some()
            && i % (4.0 / tick).round() as u32 == 0
        {
            intent.jump = true;
        }
        // Where she is while the scripts hold her controls, the first
        // few seconds of each hold (playtest 8: she hung in the air as
        // the screen went to black).
        if options.jump_with_enter {
            if game.input_locked != locked_was {
                locked_was = game.input_locked;
                locked_for = 0.0;
            }
            locked_for += tick;
            if game.input_locked
                && locked_for <= 4.0
                && i % (0.25 / tick).round() as u32 == 0
                && let Some(p) = game.scene.player.as_ref()
            {
                eprintln!(
                    "held: {locked_for:.2} s into the hold she is at {:.2}, landed {}, \
                     grounded {}, falling {:.2} m/s, driving {:?}",
                    p.character.position,
                    p.landed,
                    p.character.grounded,
                    p.character.velocity.y,
                    p.driving
                );
            }
        }
        if (!options.support.is_empty() || options.cheat_check) && game.combat.to_throw {
            intent.trigger.throw = true;
        }
        // With --bombs she drops a bomb every two seconds from a bay.
        let bomb_every = (2.0 / tick).round().max(1.0) as u32;
        if options.bombs
            && game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_some())
            && i % bomb_every == 0
        {
            intent.trigger.throw = true;
        }
        // With --shop-check she buys and calls in every support item.
        if let Some(sc) = &mut shop_check {
            let ashore = !link
                .view()
                .objectives
                .iter()
                .any(|o| o.contains("Land on the beach"))
                && game.scene.player.as_ref().is_some_and(|p| p.landed);
            if let Some(i) = sc.step(game, &mut link, ashore, tick) {
                intent = i;
            }
            if sc.finished() {
                break;
            }
        }
        // With --cheat-check, every cheat is tried once she is ashore.
        if let Some(cc) = &mut cheat_check {
            let ashore = !link
                .view()
                .objectives
                .iter()
                .any(|o| o.contains("Land on the beach"))
                && game.scene.player.as_ref().is_some_and(|p| p.landed);
            if cc.step(game, &mut link, ashore, tick) {
                break;
            }
        }
        // `--bash-target <s>`: that long after the contract, she is put
        // 1.2 m from the person the first of the scripts' markers is over,
        // facing them, and bashes them the tick after (verify-check lines).
        if let (Some(at), Some(done)) = (options.bash_target, contract_done) {
            let t = i.saturating_sub(done) as f32 * tick;
            // 20 s before: out of the HQ, onto the PMC villa's grounds below
            // it, where the layers place people (a VZ soldier by the road,
            // from earlier runs), so there is someone to try it on.
            if (t - (at - 20.0)).abs() < tick / 2.0
                && let Some(p) = game.scene.player.as_mut()
                && p.character.position.y > 300.0
            {
                p.drop_at(m2_math::glam::Vec3::new(2362.0, 0.0, -1150.0));
                eprintln!("verify-check: out of the HQ, onto the villa's grounds");
            }
            // 10 s before: the nearest placed person to her is named to the
            // scripts as `__m2_test_target` (a test driver gives them a
            // verify objective), unless a driver named one already.
            if (t - (at - 10.0)).abs() < tick / 2.0
                && link.global_guid("__m2_test_target").is_none()
                && let Some(her) = game.scene.player.as_ref().map(|p| p.character.position)
            {
                let nearest = game
                    .scene
                    .people
                    .npcs
                    .iter()
                    .filter(|n| n.alive && n.landed && !n.held)
                    // A layer's person, not one the scripts made (a
                    // starter out of her seat: guids from 0x40000000).
                    .filter_map(|n| {
                        Some((
                            n.spawn.filter(|&s| s < 0x4000_0000)?,
                            n.character.position.distance(her),
                        ))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1));
                match nearest {
                    Some((g, d)) => {
                        eprintln!("verify-check: names {g:#x} ({d:.0} m off) the test target");
                        link.run_lua(&format!("__m2_test_target = Sys.StringToGuid('{g:#x}')"));
                    }
                    None => eprintln!("verify-check: nobody placed near her to name"),
                }
            }
            // A test driver's target (`__m2_test_target`), else the
            // markers' people.
            let wanted: Vec<u32> = link
                .global_guid("__m2_test_target")
                .map(|g| vec![g])
                .unwrap_or_else(|| link.marks().iter().map(|m| m.target).collect());
            if (t - at).abs() < tick / 2.0 {
                let target = wanted.iter().find_map(|&g| {
                    let n = game
                        .scene
                        .people
                        .npcs
                        .iter()
                        .find(|n| n.alive && n.spawn == Some(g))?;
                    Some((g, n.character.position))
                });
                match (target, game.scene.player.as_mut()) {
                    (Some((g, there)), Some(p)) => {
                        let her = p.character.position;
                        let back = (her - there)
                            .with_y(0.0)
                            .normalize_or(m2_math::glam::Vec3::X);
                        p.drop_at(there + back * 1.2);
                        p.facing = (-back.x).atan2(-back.z);
                        eprintln!("verify-check: put 1.2 m from {g:#x} at {there:.1}");
                    }
                    _ => eprintln!(
                        "verify-check: none of {wanted:x?} is a person here (people's guids: {:x?})",
                        game.scene
                            .people
                            .npcs
                            .iter()
                            .filter_map(|n| n.spawn)
                            .filter(|s| *s >= 0x4000_0000)
                            .collect::<Vec<_>>()
                    ),
                }
            }
            if (t - at - 1.0).abs() < tick / 2.0 {
                if options.knock_target || options.kill_target {
                    if let Some(n) = game
                        .scene
                        .people
                        .npcs
                        .iter_mut()
                        .find(|n| n.alive && n.spawn.is_some_and(|s| wanted.contains(&s)))
                    {
                        if options.kill_target {
                            eprintln!("verify-check: {:?} is killed", n.spawn);
                            n.hurt(f32::INFINITY, m2_math::glam::Vec3::ZERO);
                        } else {
                            eprintln!("verify-check: {:?} is knocked down", n.spawn);
                            n.knock_down();
                        }
                    }
                } else {
                    eprintln!("verify-check: she bashes");
                    // Her weapons, put away by the HQ, in hand again.
                    if let Some(p) = game.scene.player.as_mut() {
                        p.holstered = false;
                    }
                    intent.trigger.melee = true;
                }
            }
        }
        // `--call-extraction <s>`: that long after the contract, the
        // support menu's extraction helicopter (the verify objective's
        // free `Extraction_*` item) is called in as a player would, its
        // designator thrown; once the helicopter has been down a while she
        // is put by it (verify-check lines).
        if let (Some(at), Some(done)) = (options.call_extraction, contract_done) {
            use crate::support::{Aim, Entry, Phase};
            let t = i.saturating_sub(done) as f32 * tick;
            match extraction {
                0 if t >= at => {
                    let index = game.support.entries().iter().position(
                        |e| matches!(e, Entry::Stocked(s) if s.key.starts_with("Extraction")),
                    );
                    match index {
                        Some(k) => {
                            eprintln!("verify-check: she calls in the extraction helicopter");
                            game.support.selected = k;
                            intent.trigger.support = true;
                            extraction = 1;
                        }
                        None => {
                            eprintln!("verify-check: no extraction helicopter in the support menu");
                            extraction = 9;
                        }
                    }
                }
                // The menu up: her pick. Her PDA open keeps it shut (closed
                // first, as `--shop-check` does), and it is asked again
                // each second till it opens.
                1 => {
                    extraction_seen += tick;
                    if game.support.phase == Phase::Menu {
                        intent.use_pressed = true;
                        extraction = 2;
                        extraction_seen = 0.0;
                    } else if game.support.pda {
                        extraction_tap = !extraction_tap;
                        intent.trigger.pda = extraction_tap;
                    } else if extraction_seen >= 1.0 {
                        eprintln!(
                            "verify-check: the support menu is {:?}; asked again",
                            game.support.phase
                        );
                        intent.trigger.support = true;
                        extraction_seen = 0.0;
                    }
                }
                2 => {
                    if game.combat.to_throw {
                        intent.trigger.throw = true;
                    }
                    if let Phase::Designating { aim, .. } = game.support.phase {
                        extraction_tap = !extraction_tap;
                        intent.trigger.fire = match aim {
                            Aim::Satellite => extraction_tap,
                            Aim::Laser => true,
                        };
                    }
                    if let Some((heli, there)) =
                        link.labelled("ExtractionHelicopter").first().copied()
                    {
                        extraction_seen += tick;
                        if extraction_seen >= 45.0
                            && let Some(p) = game.scene.player.as_mut()
                        {
                            let back = (p.character.position - there)
                                .with_y(0.0)
                                .normalize_or(m2_math::glam::Vec3::X);
                            p.drop_at(there + back * 5.0);
                            eprintln!(
                                "verify-check: put 5 m from the extraction helicopter {heli:#x} at {there:.1}"
                            );
                            extraction = 3;
                        }
                    }
                }
                _ => {}
            }
        }
        game.step(intent, tick);
        link.tick(game, tick);
        // At the scripts' transit map, autoplay picks after a moment the
        // zone a marker of theirs stands at (where the contract sends her),
        // else the first offered (invented: how she chooses).
        if auto.is_some() {
            match link.transit_open() {
                Some((seq, zones)) => {
                    transit_up += tick;
                    if transit_up > 3.0 {
                        let marked = link.marked_places();
                        let near = |at: m2_math::glam::Vec3| {
                            marked
                                .iter()
                                .map(|m| m.distance(at))
                                .fold(f32::MAX, f32::min)
                        };
                        let pick = zones
                            .iter()
                            .filter(|z| near(z.1) < 50.0)
                            .min_by(|a, b| near(a.1).total_cmp(&near(b.1)))
                            .or(zones.first())
                            .map_or(0, |z| z.0);
                        // Out of the helicopter by now (its transit done),
                        // she shuts the map: 0.
                        let pick = if game.aboard.is_some() { pick } else { 0 };
                        eprintln!(
                            "autoplay: at the transit map, of zones {:?} she picks {pick}",
                            zones.iter().map(|z| z.0).collect::<Vec<_>>()
                        );
                        link.hud_choice(seq, pick);
                        transit_up = 0.0;
                    }
                }
                None => transit_up = 0.0,
            }
        }
        game.dress(backend);
        // `--context exit,enter,talk`: each in turn, once the last is done.
        if let Some(want) = options
            .context
            .as_deref()
            .and_then(|c| c.split(',').nth(context.step))
        {
            context_check(game, &link, want, &mut context, tick);
            if context.logged.contains(&10.0) || context.logged.contains(&0.0) {
                context = ContextCheck {
                    step: context.step + 1,
                    ..ContextCheck::default()
                };
            }
        }
        // The saves the scripts ask for are written, as in play.
        for slot in link.take_saves() {
            if write_save(game, Some(&link), &save::SaveGame::slot_path(&slot)) {
                game.save_icon = game::SAVE_ICON_SECONDS;
            }
        }
        // With --placed-check, what the layers have put out is tried
        // every few seconds ([`placed::check`]).
        // Not before she is ashore: the first contract's boat ride is
        // the scripts' to end.
        if options.placed_check
            && i % (PLACED_CHECK_EVERY / tick).round() as u32 == 0
            && !link
                .view()
                .objectives
                .iter()
                .any(|o| o.contains("Land on the beach"))
            && game.scene.player.as_ref().is_some_and(|p| p.landed)
        {
            placed::check(game, &mut placed_tried, tick);
        }
        // With --props-check, every placed prop is hit once she has been
        // ashore (and in --props-area, if given) a while, for the
        // contract's layers to be in; then the run ends.
        if let Some(hit) = options.props_check
            && i % (PLACED_CHECK_EVERY / tick).round() as u32 == 0
        {
            let ashore = !link
                .view()
                .objectives
                .iter()
                .any(|o| o.contains("Land on the beach"));
            let there = game
                .scene
                .player
                .as_ref()
                .filter(|p| p.landed)
                .map(|p| p.character.position)
                .is_some_and(|at| {
                    options.props_area.is_none_or(|[x0, z0, x1, z1]| {
                        (x0..=x1).contains(&at.x) && (z0..=z1).contains(&at.z)
                    })
                });
            props_ready = if ashore && there {
                props_ready + PLACED_CHECK_EVERY
            } else {
                0.0
            };
            if props_ready > PROPS_CHECK_AFTER
                && breakables::check(game, hit, options.props_area, tick, &mut |g| {
                    // The world and the props stream in as a frame is drawn.
                    if let Err(e) = render(backend, g, fov, aspect, None) {
                        eprintln!("props-check: drawing a frame failed: {e}");
                    }
                }) > 0
            {
                break;
            }
        }
        // A hijack the scripts play on a vehicle of theirs (PmcCon004's
        // Mi35): with --autoplay its prompts are answered at once.
        if auto.is_some()
            && let Some(prompts) = link.hijack_prompts()
        {
            ride_t += tick;
            ride_pad.answer(&prompts, &mut ride_input, ride_t);
            let (pressed, held) = hijack::pad_buttons(&mut ride_input);
            link.hijack_pad(&pressed, &held);
        }
        if let Some(h) = &mut hijacks {
            if hijack_use == Some(i) {
                hijack_input.button(Action::Use, true);
            }
            h.tick(game, &mut hijack_input, tick);
            hijack_input.button(Action::Use, false);
            h.place(game);
            h.seat(game, tick);
            hijack_t += tick;
            let on_foot = game
                .scene
                .player
                .as_ref()
                .is_some_and(|p| p.driving.is_none());
            let action = link
                .marks()
                .iter()
                .filter(|m| m.kind == "action" && m.alive)
                .find_map(|m| {
                    let v = game
                        .scene
                        .vehicles
                        .iter()
                        .find(|v| v.guid == Some(m.target))?;
                    Some(v.spec.label.to_ascii_lowercase())
                });
            action_since = action.as_ref().and(action_since.or(Some(i)));
            if h.running() {
                let prompts = h.view().map(|v| v.prompts).unwrap_or_default();
                hijack_pad.answer(&prompts, &mut hijack_input, hijack_t);
                // A picture each second and a half of the hijack (eight
                // at most), with where she is in the vehicle.
                hijack_running += tick;
                if hijack_shots < 8 && hijack_running >= 1.5 * hijack_shots as f32 {
                    hijack_shots += 1;
                    render(backend, game, fov, aspect, None)?;
                    let shot = out.with_file_name(format!("{stem}-hijack{hijack_shots}.ppm"));
                    write_ppm(backend, &shot)?;
                    eprintln!(
                        "hijack shot {hijack_shots} at {hijack_running:.1} s: her in the vehicle {:.2?}",
                        h.her_in_vehicle(game)
                            .map(|(at, _, clip)| (at.to_array(), clip))
                    );
                }
            } else if auto.is_some() {
                // With --autoplay she walks up to it herself (her goal
                // for a vehicle the scripts mark), and her use key there
                // starts the hijack, as a player's does.
                let near = |g: &Game| {
                    let here = g.focus();
                    let world = g.scene.world.as_ref()?;
                    link.marks()
                        .iter()
                        .filter(|m| m.kind == "action" && m.alive)
                        .filter_map(|m| g.scene.vehicles.iter().find(|v| v.guid == Some(m.target)))
                        .filter_map(|v| {
                            let at = world.physics.vehicle_transform(v.id?)?;
                            let d = at
                                .transform_point3(m2_math::glam::Vec3::ZERO)
                                .distance(here);
                            (d < HIJACK_REACH).then(|| (v.spec.label.clone(), d))
                        })
                        .next()
                };
                if !h.loading()
                    && on_foot
                    && intent.use_pressed
                    && hijack_use.is_none_or(|u| i > u + (3.0 / tick) as u32)
                    && let Some((label, d)) = near(&*game)
                {
                    hijack_tries += 1;
                    hijack_use = Some(i + 1);
                    eprintln!(
                        "hijack: try {hijack_tries}: she presses use {d:.1} m from the {label}                          the action marker is on, {:.1} s in",
                        i as f32 * tick
                    );
                }
            } else if !h.loading()
                && on_foot
                && hijack_tries < 5
                && action_since.is_some_and(|s| i >= s + (HIJACK_WAIT / tick).round() as u32)
                && hijack_use.is_none_or(|u| i > u + (10.0 / tick) as u32)
                && let Some(label) = action
                && hijack::beside(game, h, Some(&label)).is_some()
            {
                hijack_tries += 1;
                hijack_use = Some(i + (1.0 / tick).round() as u32);
                eprintln!(
                    "hijack: try {hijack_tries} at the {label} the action marker is on, {:.1} s in",
                    i as f32 * tick
                );
            }
        }
        if options.movies {
            movies_offscreen(&mut link, install, out);
        }
        // Until the contract is done the game's HUD doesn't run here, so
        // its message box can't call back as each message ends: an
        // objective's added message, which the objective waits on before
        // going on (`fOnInitialNotesComplete`: GurCon002's cathedral
        // countdown). Each is called back once its `nDuration` is up
        // (inferred: the box shows one at a time, which isn't kept).
        if late_hud.is_none() {
            let now = i as f32 * tick;
            for c in link.view().hud_calls {
                if c.seq <= boxed_seen {
                    continue;
                }
                boxed_seen = c.seq;
                if c.callback && c.group == "MessageBox" && c.method == "AddMessage" {
                    boxed.push((now + message_seconds(&c.data), c.seq));
                }
            }
            boxed.retain(|&(due, seq)| {
                if due > now {
                    return true;
                }
                link.hud_callback(seq);
                false
            });
        }
        if late_hud.is_none() {
            let wanted = screen_cycle.wanted(game.arrival.is_some(), link.black(), tick);
            for step in screen_cycle.update(wanted, tick, false) {
                eprintln!("loading screen: {step:?} at {:.1} s", i as f32 * tick);
            }
        }
        // After the contract (--after-contract) the game's HUD runs too:
        // the flow waits on its fanfare closing.
        if contract_done.is_some() {
            let (hud, settings) = late_hud.get_or_insert_with(|| {
                let mut hud = ui::Hud::new();
                hud.game = game_hud(Some(install), &Mode::Island(None), false, game);
                hud.loading = loading_screen(Some(install));
                hud.wait = true;
                if let Some(g) = &mut hud.game {
                    g.log_to(&out.with_file_name(format!("{stem}-late-hud.log")));
                }
                (hud, Config::default())
            });
            let mut info = hud_info(game, Some(&link), fov, aspect);
            let mut state = gamehud::HudState::of(game);
            // She presses Enter every four seconds, as a player would to
            // close the contract's fanfare.
            if i % (4.0 / tick).round() as u32 == 0 {
                state.buttons.push("selection".into());
            }
            // `--dialog-keys`: with a dialog box of the scripts up, its keys
            // in turn, the first 3 s after it opened and each 2 s on.
            if link.dialog_open() {
                dialog_up += tick;
                let keys: Vec<&str> = options
                    .dialog_keys
                    .as_deref()
                    .map(|k| k.split(',').collect())
                    .unwrap_or_default();
                let due = ((dialog_up - 3.0) / 2.0).floor();
                if due >= 0.0 && (due as usize) == dialog_pressed && dialog_pressed < keys.len() {
                    let name = keys[dialog_pressed];
                    let b = match name {
                        "up" => Some("BUTTON_PAD1_U"),
                        "down" => Some("BUTTON_PAD1_D"),
                        "left" => Some("BUTTON_PAD1_L"),
                        "right" => Some("BUTTON_PAD1_R"),
                        "accept" => Some("BUTTON_PAD2_D"),
                        "back" => Some("BUTTON_PAD2_R"),
                        _ => None,
                    };
                    eprintln!(
                        "dialog-check: {:.0} s into the box, she presses {name}",
                        dialog_up
                    );
                    state.dialog_keys.extend(b);
                    dialog_pressed += 1;
                }
            } else if dialog_up > 0.0 {
                eprintln!("dialog-check: the box closed after {dialog_up:.1} s");
                dialog_up = 0.0;
            }
            info.game = Some(state);
            let (w, h) = backend.size();
            let mut input = ui::headless_input(w, h, f64::from(i) * f64::from(tick));
            // And the Enter key itself, down then up, for the movie that
            // has the controls (the fanfare's Key listener).
            let every = (4.0 / tick).round() as u32;
            if i % every <= 1 {
                input.events.push(egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: i % every == 0,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = hud.build(input, &info, settings);
            if let Some(g) = &mut hud.game {
                for seq in g.take_callbacks() {
                    eprintln!("after contract: the HUD called back call {seq}");
                    link.hud_callback(seq);
                }
                for (seq, picked) in g.take_choices() {
                    eprintln!("dialog-check: box {seq} closed on option {picked}");
                    link.hud_choice(seq, picked);
                }
            }
        }
        // `--visit-faction <abbrev>`: 30 s after the contract, she is put
        // 25 m from the nearest soldier spawn of that faction whose layer
        // is in; 15 s on, she hurts the nearest of them; who stands with
        // which side is logged every 5 s from the visit.
        if let (Some(f), Some(done)) = (options.visit_faction.as_deref(), contract_done) {
            // `Oil:watch`: she only watches, hurting nobody.
            let (f, hurts) = f.strip_suffix(":watch").map_or((f, true), |f| (f, false));
            let t = i.saturating_sub(done) as f32 * tick;
            if visit.is_none() && t >= 30.0 {
                let her = game.focus();
                // `Radio`: the nearest radio whose layer is in, 10 m off.
                let radio = f == "Radio";
                let spot = game.scene.world.as_ref().and_then(|w| {
                    if radio {
                        return w
                            .index
                            .music_sources
                            .iter()
                            .filter(|s| game.scene.people.layer_in(Some(s.layer)))
                            .min_by(|a, b| a.at.distance(her).total_cmp(&b.at.distance(her)))
                            .map(|s| (s.at, format!("radio {:#x}", s.entity)));
                    }
                    w.index
                        .spawns
                        .iter()
                        .filter(|s| game.scene.people.layer_in(s.layer) && s.ai.is_some())
                        .filter(|s| {
                            [s.template.as_deref(), Some(s.name.as_str())]
                                .into_iter()
                                .flatten()
                                .find_map(npc::faction_of)
                                == Some(f)
                        })
                        .min_by(|a, b| {
                            a.position
                                .distance(her)
                                .total_cmp(&b.position.distance(her))
                        })
                        .map(|s| (s.position, s.name.clone()))
                });
                match (spot, game.scene.player.as_mut()) {
                    (Some((at, name)), Some(p)) => {
                        let away = (her - at).with_y(0.0).normalize_or(m2_math::glam::Vec3::X);
                        let off = if radio { 10.0 } else { 25.0 };
                        p.drop_at(at + away * off);
                        eprintln!("faction-check: {f}: put {off:.0} m from {name} at {at:.1}");
                        visit = Some(0.0);
                    }
                    _ => {
                        eprintln!(
                            "faction-check: {f}: no soldier spawn of theirs in a layer that is in"
                        );
                        visit = Some(f32::INFINITY);
                    }
                }
            }
            if let Some(v) = visit.as_mut().filter(|v| v.is_finite()) {
                *v += tick;
                let her = game.focus();
                if hurts && (*v - 15.0).abs() < tick / 2.0 {
                    let near = game
                        .scene
                        .people
                        .npcs
                        .iter_mut()
                        .filter(|n| n.alive && n.faction == Some(f))
                        .min_by(|a, b| {
                            a.character
                                .position
                                .distance(her)
                                .total_cmp(&b.character.position.distance(her))
                        });
                    if let Some(n) = near {
                        n.hurt(10.0, m2_math::glam::Vec3::ZERO);
                        eprintln!(
                            "faction-check: she hurts one of them at {:.1}",
                            n.character.position
                        );
                    }
                }
                if ((*v / 5.0).fract() * 5.0) < tick && *v < 45.0 {
                    let near: Vec<&npc::Npc> = game
                        .scene
                        .people
                        .npcs
                        .iter()
                        .filter(|n| {
                            n.alive
                                && n.faction == Some(f)
                                && n.character.position.distance(her) < 120.0
                        })
                        .collect();
                    eprintln!(
                        "faction-check: {:.0} s: {} of {f} within 120 m, {} fighting her, {} on alert; pursuit {:?}; hostile {:?}",
                        *v,
                        near.len(),
                        near.iter().filter(|n| n.soldier).count(),
                        near.iter().filter(|n| n.alert).count(),
                        game.scene.people.pursuit,
                        game.scene.people.hostile
                    );
                }
                // And a picture with the game's HUD (its faction meters)
                // 40 s on.
                if (*v - 40.0).abs() < tick / 2.0
                    && let Some((hud, _)) = late_hud.as_mut()
                {
                    render(backend, game, fov, aspect, Some(hud))?;
                    write_ppm(backend, &picture("faction40".into()))?;
                }
            }
        }
        // `--medevac <s>`: that long after the contract, the pause menu's
        // Medevac as confirmed, if the scripts let her have it then.
        if let (Some(at), Some(done)) = (options.medevac, contract_done)
            && !medevac_done
            && i.saturating_sub(done) as f32 * tick >= at
        {
            medevac_done = true;
            let state = link.medevac_state();
            eprintln!(
                "medevac-check: at {:.1} the scripts answer {state:?} (can, cost); cash {:.0}",
                game.focus(),
                link.view().cash.unwrap_or(0.0)
            );
            if state.is_some_and(|(can, _)| can) {
                link.medevac();
                medevac_from = Some((0.0, game.focus()));
            }
        }
        if let Some((t, from)) = &mut medevac_from {
            *t += tick;
            for after in [1.0f32, 10.0, 30.0] {
                if (*t - after).abs() < tick / 2.0 {
                    eprintln!(
                        "medevac-check: {after:.0} s after (from {from:.1}) she is at {:.1}, cash {:.0}",
                        game.focus(),
                        link.view().cash.unwrap_or(0.0)
                    );
                }
            }
        }
        // After the contract (--after-contract), where she is every five
        // seconds.
        if contract_done.is_some() && i % (5.0 / tick).round() as u32 == 0 {
            let view = link.view();
            eprintln!(
                "after contract: {:.0} s, she is at {:.1}, health {:.0}, objectives {:?}",
                i as f32 * tick,
                game.focus(),
                game.combat.hp,
                view.objectives
            );
            eprintln!(
                "after contract tally {:.0} s: {}; {}; {}",
                i as f32 * tick,
                game.combat.tally.line(game.combat.fired.rifle),
                game.scene.people.idle.line(),
                game.scene.people.fight_summary()
            );
            eprintln!(
                "after contract checks {:.0} s: {}",
                i as f32 * tick,
                game.scene.people.checks.line()
            );
        }
        // And the strike is pictured 2 s and 6 s after its first ordnance
        // goes off (`-strike`, `-strike-late`).
        if !options.support.is_empty() {
            let strikes = &game.scene.strikes;
            if strikes.went_off > 0 && strike_seen.is_none() {
                strike_seen = Some(i);
                eprintln!(
                    "support: first ordnance went off at {:.1}, {:.1} s in",
                    strikes.first.unwrap_or_default(),
                    i as f32 * tick
                );
            }
            for (after, name) in [(2.0, "strike"), (6.0, "strike-late")] {
                if strike_seen.is_some_and(|s| i == s + (after / tick) as u32) {
                    let path = picture(name.into());
                    hud_picture(backend, game, &link, fov, aspect, &path)?;
                    eprintln!(
                        "support: pictured her view {after} s after the first blast ({} gone off), health {:.0}: {}",
                        game.scene.strikes.went_off,
                        game.combat.hp,
                        path.display()
                    );
                }
            }
        }
        if auto.is_some() {
            let shown = link.view().tutorial;
            if shown != tutorial_shown {
                eprintln!(
                    "autoplay: the tutorial box reads {shown:?} at {:.1} s",
                    i as f32 * tick
                );
                tutorial_shown = shown;
            }
        }
        for c in game.scene.take_cues(game.focus(), CUE_REACH) {
            let e = heard.entry((c.category, c.name)).or_default();
            if c.hers {
                e.0 += 1;
            } else {
                e.1 += 1;
            }
        }
        // Lost (round 88 went on 40 minutes at NaN): nothing more to see.
        if auto.is_some() && !game.focus().is_finite() {
            eprintln!(
                "autoplay: her position is not a number after {:.1} s of play: stopping",
                i as f32 * tick
            );
            break;
        }
        if i % every == 0 {
            render(backend, game, fov, aspect, None)?;
        }
        if auto.is_some() {
            for (copter, at, to) in game.scene.flights.hovering() {
                if pictured.insert(copter) {
                    let n = pictured.len();
                    let look = (at + to) * 0.5;
                    let path = picture(format!("delivery{n}"));
                    let eye = delivery_picture(backend, game, fov, aspect, to, look, None, &path)?;
                    let vehicles = &game.scene.vehicles;
                    for (l, hangs, found) in game.scene.flights.hung_from(copter, vehicles) {
                        match found.map(|i| &vehicles[i]) {
                            Some(v) => eprintln!(
                                "delivery picture: {} ({:#x}) hangs at {hangs:.1}, {:.1} m from the eye, \
                                 its box {:.2}..{:.2}{}",
                                v.spec.label,
                                l.guid,
                                hangs.distance(eye),
                                v.model.bounds.0,
                                v.model.bounds.1,
                                if v.id.is_some() { ", also in play" } else { "" }
                            ),
                            None if l.model.is_some() => eprintln!(
                                "delivery picture: crate {:#x} hangs at {hangs:.1}",
                                l.guid
                            ),
                            None => eprintln!(
                                "delivery picture: vehicle {:#x} hangs at {hangs:.1} but has not loaded: not drawn",
                                l.guid
                            ),
                        }
                    }
                    // Hovering, letting go and four seconds' fall and
                    // settling.
                    down.push((i + (6.0 / tick) as u32, n, eye, to));
                }
            }
            let due: Vec<_> = down.iter().filter(|d| d.0 <= i).copied().collect();
            down.retain(|d| d.0 > i);
            for (_, n, eye, to) in due {
                let path = picture(format!("delivery{n}-down"));
                let look = to + m2_math::glam::Vec3::Y;
                delivery_picture(backend, game, fov, aspect, to, look, Some(eye), &path)?;
            }
            let in_boat = game
                .scene
                .player
                .as_ref()
                .and_then(|p| p.driving)
                .and_then(|v| game.scene.vehicles.get(v))
                .is_some_and(|v| v.spec.is_boat());
            if in_boat {
                let since = *seated.get_or_insert(i);
                let planing = game.driving().is_some_and(|(_, s)| s >= PLANE_SPEED);
                if boat_shot.is_none() && (planing || i >= since + (20.0 / tick) as u32) {
                    boat_shot = Some(i);
                }
            } else if seated.is_some() {
                ashore_shot.get_or_insert(i + (3.0 / tick) as u32);
            }
            // Not what the layers place: hundreds stand from the start.
            for p in game.scene.flights.pickups() {
                if !game.placed.is_pickup(p.from) && opened.insert(p.key()) {
                    // Two seconds on, when the soldier who left it has
                    // fallen.
                    pickup_shots.push((i + (2.0 / tick) as u32, p.key(), opened.len()));
                }
            }
            let due: Vec<((u32, u32), usize)> = pickup_shots
                .iter()
                .filter(|s| s.0 == i)
                .map(|s| (s.1, s.2))
                .collect();
            for (key, n) in due {
                let lying = game
                    .scene
                    .flights
                    .pickups()
                    .iter()
                    .find(|p| p.key() == key)
                    .copied();
                let Some(p) = lying else {
                    eprintln!(
                        "pickup picture: the {:#x} the crate {:#x} left was taken first",
                        key.1, key.0
                    );
                    continue;
                };
                let away = (p.at - game.focus())
                    .with_y(0.0)
                    .try_normalize()
                    .unwrap_or(m2_math::glam::Vec3::X);
                // Steeply from above, so that a body by it doesn't hide it.
                let eye = p.at + away * 1.5 + m2_math::glam::Vec3::Y * 3.5;
                delivery_picture(
                    backend,
                    game,
                    fov,
                    aspect,
                    p.at,
                    p.at,
                    Some(eye),
                    &picture(format!("pickup{n}")),
                )?;
                eprintln!(
                    "pickup picture: {:#x} lies at {:.1}, its model {}",
                    p.what.template,
                    p.at,
                    match p.what.model {
                        Some(m) if game.scene.flights.has_model(m) => format!("{m:08x} drawn"),
                        Some(m) => format!("{m:08x} not loaded: not drawn"),
                        None => "none".into(),
                    }
                );
            }
            let timing = matches!(game.support.phase, support::Phase::Timing { .. });
            if timing && !wheel_up {
                satellite_shots += 1;
                let path = picture(format!("satellite{satellite_shots}"));
                hud_picture(backend, game, &link, fov, aspect, &path)?;
                eprintln!(
                    "autoplay: pictured the satellite's view over {:.1} from her at {:.1}: {}",
                    game.support.looking().unwrap_or_default(),
                    game.focus(),
                    path.display()
                );
            }
            wheel_up = timing;
            if game.support.pda && !pda_up {
                pda_shots += 1;
                let path = picture(format!("pda{pda_shots}"));
                hud_picture(backend, game, &link, fov, aspect, &path)?;
                eprintln!(
                    "autoplay: pictured her PDA open, the tutorial line {:?}: {}",
                    link.view().tutorial,
                    path.display()
                );
            }
            pda_up = game.support.pda;
            if let Some(at) = game.support.landed.iter().find_map(|l| match l {
                support::Impact::Bomb(at) => Some(*at),
                _ => None,
            }) {
                bombs += 1;
                for after in [2, 8] {
                    bomb_shots.push((i + (after as f32 / tick) as u32, bombs, after, at));
                }
            }
            let due: Vec<(usize, u32, m2_math::glam::Vec2)> = bomb_shots
                .iter()
                .filter(|s| s.0 == i)
                .map(|s| (s.1, s.2, s.3))
                .collect();
            for (n, after, at) in due {
                let late = if after > 2 { "-late" } else { "" };
                let path = picture(format!("bomb{n}{late}"));
                hud_picture(backend, game, &link, fov, aspect, &path)?;
                eprintln!(
                    "autoplay: pictured her view {after} s after bomb {n}, from {:.1}, health {:.0}: {}",
                    game.focus(),
                    game.combat.hp,
                    path.display()
                );
                describe_bombed(game, at, fov, aspect);
            }
            for (due, name) in [(boat_shot, "boat"), (ashore_shot, "ashore")] {
                if due == Some(i) {
                    render(backend, game, fov, aspect, None)?;
                    write_ppm(backend, &picture(name.into()))?;
                    match game.driving() {
                        Some((_, speed)) => eprintln!(
                            "autoplay: pictured {name} at {:.1} going {speed:.1} m/s, {:.1} s after she was seated",
                            game.focus(),
                            (i - seated.unwrap_or(i)) as f32 * tick
                        ),
                        None => eprintln!("autoplay: pictured {name} at {:.1}", game.focus()),
                    }
                }
            }
        }
        if auto.is_some() && link.contracts_completed() > contracts_seen {
            contracts_seen = link.contracts_completed();
            eprintln!(
                "autoplay: contract {contracts_seen} completed after {:.1} s of play ({:.0} s here)",
                i as f32 * tick,
                played.elapsed().as_secs_f32()
            );
            contract_done.get_or_insert(i);
            // She stops playing once the last asked for is done; the
            // scripts go on (--after-contract).
            if contracts_seen >= options.contracts.max(1) {
                play_done = Some(i);
            }
        }
        // The job she was skipped to, done (a set job or a collection
        // completed, a standing bounty paid): she stops as for a contract.
        if auto.is_some()
            && play_done.is_none()
            && let Some(job) = link.job().filter(|j| j.finished())
        {
            eprintln!(
                "autoplay: job {} done after {:.1} s of play ({:.0} s here): {} counted{}, cash {:.0}",
                job.id,
                i as f32 * tick,
                played.elapsed().as_secs_f32(),
                job.view.counted,
                if job.view.completed {
                    ", completed"
                } else {
                    ""
                },
                link.resources().unwrap_or_default().0
            );
            play_done = Some(i);
        }
        if play_done.is_some_and(|d| i >= d + (options.after_contract / tick).round() as u32) {
            break;
        }
    }
    if let Some(sc) = &shop_check {
        for line in sc.summary() {
            eprintln!("{line}");
        }
        let (cash, fuel) = link.resources().unwrap_or_default();
        eprintln!("shop-check: she ends with cash {cash:.0}, fuel {fuel:.0}");
    }
    if auto.is_some() {
        let mut sounds = sound::ClipSounds::load(install);
        let rows: Vec<String> = heard
            .iter()
            .map(|((category, name), (hers, others))| {
                format!(
                    "{category}/{name} {hers}+{others} ({} sounds)",
                    sounds.sounds(name).len()
                )
            })
            .collect();
        eprintln!(
            "clip cues met (hers+others'; the sounds they play of {}): {}",
            sounds.len(),
            rows.join(", ")
        );
    }
    if let Some(a) = &auto {
        eprintln!(
            "autoplay: {}{}",
            a.summary(),
            if link.completed() {
                ""
            } else {
                "; the contract was not completed"
            }
        );
        look_at_story_character(game);
    }
    while let Some(world) = &game.scene.world {
        let waiting = game.scene.player.as_ref().is_some_and(|p| !p.landed);
        if (world.stats().pending == 0 && !waiting) || started.elapsed() > Duration::from_secs(400)
        {
            break;
        }
        game.step(Intent::default(), tick);
        link.tick(game, tick);
        game.dress(backend);
        render(backend, game, fov, aspect, None)?;
    }
    let view = link.view();
    eprintln!(
        "mission: player at {:.1}; objectives {:?}; {} markers; message {:?}; status {:?}",
        game.focus(),
        view.objectives,
        view.markers.len(),
        view.message,
        view.status
    );
    for (label, at) in mission::MissionLink::vehicles(game) {
        match at {
            Some(at) => eprintln!("mission vehicle {label} at {at:.1}"),
            None => eprintln!("mission vehicle {label} not placed"),
        }
    }
    for line in link.report(game) {
        eprintln!("mission: {line}");
    }
    Ok(link)
}

/// Turns the camera onto the nearest story character within 40 m (the
/// last picture shows them), saying whether anything stands between.
fn look_at_story_character(game: &mut Game) {
    let here = game.focus();
    let people = &game.scene.people;
    let Some((name, chest)) = people
        .npcs
        .iter()
        .filter(|n| n.held)
        .map(|n| (people.kinds[n.kind].role.clone(), n.chest()))
        .filter(|(_, c)| c.distance(here) < 40.0)
        .min_by(|a, b| a.1.distance(here).total_cmp(&b.1.distance(here)))
    else {
        eprintln!("autoplay: no story character within 40 m");
        return;
    };
    aim_at(game, chest);
    let (Some(world), Some(p)) = (&game.scene.world, &game.scene.player) else {
        return;
    };
    let eye = p.eye();
    let to = chest - eye;
    let between = world
        .physics
        .cast_ray_keyed(eye, to, to.length())
        .map(|(d, _, key)| format!("{} at {d:.1} m", static_name(world, key)));
    eprintln!(
        "autoplay: looking at {name} at {chest:.1}, {:.1} m from the camera; in the way: {}",
        to.length(),
        between.as_deref().unwrap_or("nothing")
    );
}

/// Waits for the parked vehicles, gets in the nearest and drives it
/// forward for four seconds (backing out first if it is parked close
/// behind something, and steering round what it meets), printing how it
/// went.
/// With `--screenshot --drive --hijack`: puts her beside the nearest
/// hijackable vehicle (`--vehicle`'s, if given), starts the hijack and
/// answers its prompts at once ([`hijack::Autopad`]), and leaves her in the
/// picture where it ends: driving it, or let go beside it.
fn hijack_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    options: &Options,
    install: &Install,
) -> anyhow::Result<()> {
    let tick = TICK.as_secs_f32();
    let mut hijacks = hijack::Hijacks::start(install, options.hijack_crew.clone());
    let mut input = Input::default();
    let started = Instant::now();
    // The game's HUD, for the buttons the scripts put up.
    let mut hud = ui::Hud::new();
    hud.game = game_hud(Some(install), &Mode::Island(None), options.old_hud, game);
    hud.wait = true;
    if let (Some(g), Some(out)) = (&mut hud.game, &options.screenshot) {
        g.log_to(&out.with_file_name(format!(
            "{}-hud.log",
            out.file_stem().and_then(|s| s.to_str()).unwrap_or("hijack")
        )));
    }
    let mut settings = Config::default();
    while hijacks.loading() && started.elapsed() < Duration::from_secs(120) {
        hijacks.tick(game, &mut input, tick);
        game.step(Intent::default(), tick);
    }
    eprintln!(
        "hijack: scripts {} after {:.1} s",
        if hijacks.loading() {
            "still loading"
        } else {
            "loaded"
        },
        started.elapsed().as_secs_f32()
    );
    let Some(i) = hijack::beside(game, &mut hijacks, options.vehicle.as_deref()) else {
        eprintln!("hijack: no hijackable vehicle with a driver parked");
        return Ok(());
    };
    // Let her land.
    for _ in 0..60 {
        hijacks.tick(game, &mut input, tick);
        game.step(Intent::default(), tick);
    }
    // `--grapple`: from 40 m off, facing it, she hooks the helicopter and
    // is pulled to its entrance, which starts its hijack.
    if options.grapple && !grapple_in(game, &mut hijacks, &mut input, i, tick) {
        return Ok(());
    }
    let Some(view) = hijacks.view() else {
        eprintln!(
            "hijack: no prompt beside the {}",
            game.scene.vehicles[i].spec.label
        );
        return Ok(());
    };
    eprintln!("hijack: prompt \"Hijack {}\"", view.vehicle);
    // As a player walks up, the crewman's model streams in.
    let waiting = Instant::now();
    while !hijacks.crew_in(game) && waiting.elapsed() < Duration::from_secs(60) {
        let focus = game.focus();
        game.scene.update(backend, focus)?;
        hijacks.tick(game, &mut input, tick);
        game.step(Intent::default(), tick);
    }
    eprintln!(
        "hijack: crewman {} after {:.1} s",
        if hijacks.crew_in(game) {
            "in"
        } else {
            "not in"
        },
        waiting.elapsed().as_secs_f32()
    );
    // The HUD up before she starts (its scripts load in the background).
    if hud.game.is_some() {
        let mut info = hud_info(game, None, fov, aspect);
        info.game = Some(gamehud::HudState::of(game));
        let (w, h) = backend.size();
        let _ = hud.build(ui::headless_input(w, h, 0.0), &info, &mut settings);
    }
    input.button(Action::Use, true);
    let mut pad = hijack::Autopad::default();
    let mut t = 0.0;
    let mut shots = 0;
    // Pictures of the buttons the scripts put up: just after each shows,
    // and a second on (a tap's prompt stays up while she taps).
    let mut button_shots: Vec<f32> = Vec::new();
    let mut button_shot = 0;
    loop {
        hijacks.tick(game, &mut input, tick);
        input.button(Action::Use, false);
        game.step(Intent::default(), tick);
        hijacks.place(game);
        hijacks.seat(game, tick);
        t += tick;
        if hud.game.is_some() {
            let mut info = hud_info(game, None, fov, aspect);
            let mut state = gamehud::HudState::of(game);
            state.gui_calls = hijacks.take_gui_calls();
            for c in &state.gui_calls {
                eprintln!("hijack: t={t:.1} HUD call {}.{}", c.module, c.method);
                if c.method == "ShowButton" {
                    button_shots.extend([t + 0.1, t + 1.0]);
                }
            }
            info.game = Some(state);
            let (w, h) = backend.size();
            let _ = hud.build(ui::headless_input(w, h, f64::from(t)), &info, &mut settings);
        }
        // A picture every two seconds of it, beside the one at the end.
        if t >= 2.0 * (shots + 1) as f32 && options.screenshot.is_some() {
            shots += 1;
            if let Some((her, (lo, hi), body)) = hijacks.her_in_vehicle(game) {
                eprintln!(
                    "hijack: t={t:.1} her feet in the vehicle's space {:.2?} (its box {:.2?} to {:.2?}) {body}",
                    her.to_array(),
                    lo.to_array(),
                    hi.to_array()
                );
            }
            render(backend, game, fov, aspect, Some(&mut hud))?;
            if let Some(out) = &options.screenshot {
                let path = out.with_file_name(format!(
                    "{}-{shots:02}.ppm",
                    out.file_stem().and_then(|s| s.to_str()).unwrap_or("hijack")
                ));
                write_ppm(backend, &path)?;
            }
        }
        if let Some(k) = button_shots.iter().position(|&at| t >= at) {
            button_shots.remove(k);
            button_shot += 1;
            render(backend, game, fov, aspect, Some(&mut hud))?;
            if let Some(out) = &options.screenshot {
                let path = out.with_file_name(format!(
                    "{}-button{button_shot:02}.ppm",
                    out.file_stem().and_then(|s| s.to_str()).unwrap_or("hijack")
                ));
                eprintln!("hijack: t={t:.1} button picture {}", path.display());
                write_ppm(backend, &path)?;
            }
        }
        if !hijacks.running() || t > 60.0 {
            break;
        }
        let prompts = hijacks.view().map(|v| v.prompts).unwrap_or_default();
        pad.answer(&prompts, &mut input, t);
    }
    let driving = game.scene.player.as_ref().and_then(|p| p.driving) == Some(i);
    eprintln!("hijack: over after {t:.1} s; driving it: {driving}");
    render(backend, game, fov, aspect, None)?;
    Ok(())
}

/// Where `--context` is: seconds since she was put at the action's object,
/// and since she took it (each `None` until then).
#[derive(Debug, Default)]
struct ContextCheck {
    put: Option<f32>,
    taken: Option<(f32, m2_math::glam::Vec3)>,
    /// Her place was logged this long after taking it (0: never offered).
    logged: Vec<f32>,
    /// Times she was put by it again, moved away from it.
    moved: u32,
    /// Which of the comma-separated actions asked for this is.
    step: usize,
}

/// The pad button a key stands for in a dialog box the scripts put up
/// (`MrxGuiDialogBox._HandleInputEvent` reads the pad's d-pad or left
/// stick up and down, A to take the option, B to back out; the wager's
/// `MrxGuiNumericBox` left and right too, between its digits and its
/// buttons): the arrows or WASD step, Enter, Space or E take, Escape or
/// Backspace back out. Which keys is invented (the PC's bindings for the
/// boxes aren't read).
fn dialog_button(code: KeyCode) -> Option<&'static str> {
    Some(match code {
        KeyCode::ArrowUp | KeyCode::KeyW => "BUTTON_PAD1_U",
        KeyCode::ArrowDown | KeyCode::KeyS => "BUTTON_PAD1_D",
        KeyCode::ArrowLeft | KeyCode::KeyA => "BUTTON_PAD1_L",
        KeyCode::ArrowRight | KeyCode::KeyD => "BUTTON_PAD1_R",
        KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space | KeyCode::KeyE => "BUTTON_PAD2_D",
        KeyCode::Escape | KeyCode::Backspace => "BUTTON_PAD2_R",
        _ => return None,
    })
}

/// `--grapple` with `--screenshot --drive --hijack`: puts her 40 m from
/// helicopter `i`'s entrance, lets her land, fires the grappling hook at
/// it as if aimed and keyed, and runs the line until the helicopter's
/// hijack starts (logging the hook's flight, the pull and the hand-off).
/// False if it never starts.
fn grapple_in(
    game: &mut Game,
    hijacks: &mut hijack::Hijacks,
    input: &mut Input,
    i: usize,
    tick: f32,
) -> bool {
    use m2_math::glam::Vec3;
    let label = game.scene.vehicles[i].spec.label.clone();
    let Some(door) = grapple::entrance(game, i) else {
        eprintln!("grapple: the {label} is no helicopter");
        return false;
    };
    if let Some(p) = game.scene.player.as_mut() {
        let here = p.character.position;
        let away = (here - door).with_y(0.0).normalize_or(Vec3::X);
        p.character.position = (door + away * 40.0).with_y(here.y);
        p.character.velocity = Vec3::ZERO;
        p.landed = false;
    }
    for _ in 0..90 {
        hijacks.tick(game, input, tick);
        game.step(Intent::default(), tick);
    }
    let Some(her) = game.scene.player.as_ref().map(|p| p.character.position) else {
        return false;
    };
    eprintln!(
        "grapple: {:.1} m from the {label}'s entrance ({:.1} m up)",
        her.distance(door),
        door.y - her.y
    );
    let mut g = std::mem::take(&mut game.grapple);
    g.enabled = true;
    g.aim(game, door - her);
    let fired = g.fire();
    game.grapple = g;
    if !fired {
        eprintln!("grapple: the {label} is not in her sights within reach");
        return false;
    }
    eprintln!("grapple: hook fired at the {label}");
    for _ in 0..(20.0 / tick) as usize {
        hijacks.tick(game, input, tick);
        if hijacks.running() {
            eprintln!("grapple: the {label}'s hijack started");
            return true;
        }
        game.step(Intent::default(), tick);
    }
    eprintln!("grapple: no hijack started in 20 s");
    false
}

/// The grappling hook's keys, each frame on foot ([`grapple`]): with the
/// grapple `allowed`, the helicopter in her sights is picked; the use key
/// fires the hook at it, or, on the line, lets go ("Stop grappling").
fn grapple_keys(game: &mut Game, input: &mut Input, allowed: bool) {
    let mut g = std::mem::take(&mut game.grapple);
    g.enabled = allowed;
    if g.grappling() {
        if input.take_pressed(Action::Use) {
            eprintln!("grapple: she lets go of the line");
            g.release();
        }
    } else {
        let yaw = game.view_heading();
        g.aim(game, m2_math::glam::Vec3::new(yaw.sin(), 0.0, yaw.cos()));
        if g.target.is_some() && input.take_pressed(Action::Use) {
            let label = g
                .target
                .and_then(|i| game.scene.vehicles.get(i))
                .map(|v| v.spec.label.clone())
                .unwrap_or_default();
            if g.fire() {
                eprintln!("grapple: hook fired at the {label}");
            }
        }
    }
    game.grapple = g;
}

/// How long the message box shows a message (its call's `nDuration`,
/// seconds; 5 when not given, as the objectives' messages ask).
fn message_seconds(data: &gamehud::Plain) -> f32 {
    use gamehud::Plain;
    let Plain::Table(t) = data else {
        return 5.0;
    };
    t.iter()
        .find_map(|(k, v)| match (k, v) {
            (Plain::Text(k), Plain::Number(n)) if k == "nDuration" => Some(*n as f32),
            _ => None,
        })
        .unwrap_or(5.0)
}

/// With `--context`: once the scripts offer a context action whose prompt
/// reads `want`, she is put 1.5 m from its object; once the game offers it
/// her (a second on), she takes it as the use key does. Her place is
/// logged 1, 4 and 10 s after (`context-check:` lines): an HQ exit takes
/// her out of the interior, 450 m up, to the villa's grounds.
fn context_check(
    game: &mut Game,
    link: &mission::MissionLink,
    want: &str,
    c: &mut ContextCheck,
    tick: f32,
) {
    use m2_math::glam::Vec3;
    let here = game.scene.player.as_ref().map(|p| p.character.position);
    if let Some((since, from)) = &mut c.taken {
        *since += tick;
        for after in [1.0, 4.0, 10.0] {
            if *since >= after && !c.logged.contains(&after) {
                c.logged.push(after);
                eprintln!(
                    "context-check: {after:.0} s after taking {want:?} (at {from:.1}) she is at {:.1}",
                    here.unwrap_or(Vec3::NAN)
                );
            }
        }
        return;
    }
    let Some((g, at)) = link.context_spot(want) else {
        return;
    };
    // A person's action is where they stand in play (Fiona's starter is
    // placed 45 m off her desk, over no floor).
    let at = game
        .scene
        .people
        .npcs
        .iter()
        .find(|n| n.spawn == Some(g))
        .map_or(at, |n| n.character.position);
    match &mut c.put {
        None => {
            if let Some(p) = game.scene.player.as_mut() {
                // 0.6 m short of it, from where she stands (the wardrobe's
                // reaches 1 m): a starter sits at
                // a desk she would be put into.
                let from = (p.character.position - at)
                    .with_y(0.0)
                    .normalize_or(Vec3::X);
                p.character.position = at + from * 0.6;
                p.character.velocity = Vec3::ZERO;
            }
            eprintln!("context-check: {want:?} offered at {g:#x} ({at:.1}): she is put by it");
            c.put = Some(0.0);
        }
        Some(t) => {
            *t += tick;
            let offered = game.context.as_ref().filter(|(o, _)| *o == g);
            // Soon: put by a starter whose spot has no floor under it, she
            // falls out of its reach within the second.
            if *t >= 0.2
                && let Some((_, text)) = offered
            {
                eprintln!("context-check: the prompt reads {text:?}; she takes it");
                game.context_taken = Some(g);
                c.taken = Some((0.0, here.unwrap_or(at)));
            } else if *t >= 2.0 && here.is_some_and(|h| h.distance(at) > 5.0) && c.moved < 5 {
                // Moved away by the scripts (the move into the HQ came
                // after the action was offered): put by it again.
                c.moved += 1;
                c.put = None;
            } else if *t >= 10.0 && c.logged.is_empty() {
                c.logged.push(0.0);
                eprintln!(
                    "context-check: not offered her in 10 s (she is at {:.1}; offered {:?})",
                    here.unwrap_or(Vec3::NAN),
                    game.context
                );
            }
        }
    }
}

/// Parks the vehicle she drives 12 m short of the nearest placed object
/// of `kind` (within 150 m) that vehicles break or shove: "fence" (broken
/// by a heavy enough vehicle), "plant" (flattened), "lamp" (knocked over)
/// or "box" (loose, no such property), facing it. True if it found one.
fn plough_at(game: &mut Game, kind: &str) -> bool {
    use m2_engine::durability::CrashHow;
    use m2_math::glam::{Quat, Vec3};
    let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving) else {
        return false;
    };
    let Some(world) = game.scene.world.as_mut() else {
        return false;
    };
    let Some(id) = game.scene.vehicles[i].id else {
        return false;
    };
    let Some(here) = world
        .physics
        .vehicle_transform(id)
        .map(|t| t.w_axis.truncate())
    else {
        return false;
    };
    // `prop:<entity>`: a state layer's prop (Solano's villa gate,
    // `prop:a3b32`), put up here as its layer would.
    if let Some(e) = kind
        .strip_prefix("prop:")
        .and_then(|h| u32::from_str_radix(h.trim_start_matches("0x"), 16).ok())
    {
        let Some(placed) = world.index.state_objects.get(&e).copied() else {
            eprintln!("plough: {e:#x} is no state layer's prop");
            return false;
        };
        let at = placed.position;
        // Straight at it along its own facing (its z axis: a gate's way
        // through), from whichever side she is.
        let axis = (placed.rotation * Vec3::Z)
            .with_y(0.0)
            .normalize_or(Vec3::X);
        let back = if (here - at).dot(axis) >= 0.0 {
            axis
        } else {
            -axis
        };
        let spot = at + back * 14.0;
        let ground = world
            .physics
            .island_below(spot.with_y(spot.y + 200.0), 400.0)
            .unwrap_or(spot.y);
        let ahead = -back;
        let turn = Quat::from_rotation_y(ahead.x.atan2(ahead.z));
        world.physics.remove_vehicle(id);
        let v = &mut game.scene.vehicles[i];
        v.id = Some(
            world
                .physics
                .add_vehicle_turned(&v.spec, spot.with_y(ground + 1.0), turn),
        );
        let label = v.spec.label.clone();
        game.scene.props.note_layer([e], "--plough");
        let shown = game.scene.show_props([e]);
        eprintln!(
            "plough: prop {e:#x} (model {:08x}, template {:08x}) at {at:.1}, put up {shown}; \
             the {label} parked 14 m short at {:.1}, facing it",
            placed.model,
            placed.template,
            spot.with_y(ground)
        );
        return true;
    }
    // `car`: the nearest other vehicle parked (within 150 m), to crash
    // into ([`crate::combat::Combat::vehicle_crashes`]).
    if kind == "car" {
        let Some((j, at)) = game
            .scene
            .vehicles
            .iter()
            .enumerate()
            .filter(|&(j, v)| j != i && !v.wrecked)
            .filter_map(|(j, v)| {
                let at = world.physics.vehicle_transform(v.id?)?.w_axis.truncate();
                Some((j, at))
            })
            .filter(|(_, at)| at.distance(here) < 150.0)
            .min_by(|a, b| a.1.distance(here).total_cmp(&b.1.distance(here)))
        else {
            eprintln!("plough: no other vehicle within 150 m of {here:.1}");
            return false;
        };
        let back = (here - at).with_y(0.0).normalize_or(Vec3::X);
        let spot = at + back * 14.0;
        let ground = world
            .physics
            .island_below(spot.with_y(spot.y + 200.0), 400.0)
            .unwrap_or(spot.y);
        let ahead = -back;
        let turn = Quat::from_rotation_y(ahead.x.atan2(ahead.z));
        world.physics.remove_vehicle(id);
        let v = &mut game.scene.vehicles[i];
        v.id = Some(
            world
                .physics
                .add_vehicle_turned(&v.spec, spot.with_y(ground + 1.0), turn),
        );
        let (mine, theirs) = (
            v.spec.label.clone(),
            game.scene.vehicles[j].spec.label.clone(),
        );
        eprintln!("plough: the {mine} parked 14 m short of the {theirs} at {at:.1}, facing it");
        return true;
    }
    let wanted = |w: &m2_engine::World, o: usize| {
        let t = w.index.objects[o].template;
        let crash = w.index.template_db.crashable.get(&t).map(|c| c.how);
        match kind {
            "fence" => crash == Some(CrashHow::Breaks),
            "plant" => crash == Some(CrashHow::Flattened),
            "lamp" => crash == Some(CrashHow::KnockedOver),
            "box" => crash.is_none() && w.loose_mass(o).is_some(),
            _ => crash.is_some(),
        }
    };
    let Some((o, d)) = world
        .near(here, 150.0)
        .into_iter()
        .filter(|&(o, _)| wanted(world, o))
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        eprintln!("plough: no {kind} within 150 m of {here:.1}");
        return false;
    };
    let target = world.index.objects[o];
    let back = (here - target.position).with_y(0.0).normalize_or(Vec3::X);
    let spot = target.position + back * 12.0;
    let ground = world
        .physics
        .island_below(spot.with_y(spot.y + 200.0), 400.0)
        .unwrap_or(spot.y);
    let ahead = -back;
    let turn = Quat::from_rotation_y(ahead.x.atan2(ahead.z));
    world.physics.remove_vehicle(id);
    let v = &mut game.scene.vehicles[i];
    v.id = Some(
        world
            .physics
            .add_vehicle_turned(&v.spec, spot.with_y(ground + 1.0), turn),
    );
    eprintln!(
        "plough: {kind}: object {o} (model {:08x}, template {:08x}, crash {:?}, loose {:?}) at \
         {:.1}, {d:.1} m off; the {} parked 12 m short at {:.1}, facing it",
        target.model,
        target.template,
        world.index.template_db.crashable.get(&target.template),
        world.loose_mass(o),
        target.position,
        v.spec.label,
        spot.with_y(ground)
    );
    true
}

fn drive_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    options: &Options,
    install: Option<&Install>,
) -> anyhow::Result<()> {
    let tick = TICK.as_secs_f32();
    let started = Instant::now();
    loop {
        game.step(Intent::default(), tick);
        render(backend, game, fov, aspect, None)?;
        if (!game.scene.vehicles_loading() && game.parking_done())
            || started.elapsed() > Duration::from_secs(300)
        {
            break;
        }
    }
    eprintln!(
        "vehicles: {} parked after {:.1} s",
        game.scene
            .vehicles
            .iter()
            .filter(|v| v.id.is_some())
            .count(),
        started.elapsed().as_secs_f32()
    );
    // Let them settle on their suspension.
    for _ in 0..90 {
        game.step(Intent::default(), tick);
    }
    // A vehicle asked for that isn't parked here: one is parked.
    if let Some(want) = &options.vehicle {
        let have = |game: &Game| {
            game.scene.vehicles.iter().any(|v| {
                v.id.is_some() && v.spec.label.to_ascii_lowercase().contains(want.as_str())
            })
        };
        if !have(game) {
            let models: Vec<m2_engine::CatalogueEntry> = game
                .scene
                .catalogue
                .as_ref()
                .map(|c| {
                    c.vehicles
                        .iter()
                        .filter(|e| e.label.to_ascii_lowercase().contains(want.as_str()))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            eprintln!(
                "drive: no {want} parked; parking one of [{}]",
                models
                    .iter()
                    .map(|e| format!("{} ({}, model {:08x})", e.label, e.kind, e.model))
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            if !models.is_empty() {
                game.park_more(models);
                let waited = Instant::now();
                while !have(game) && waited.elapsed() < Duration::from_secs(90) {
                    game.step(Intent::default(), tick);
                    render(backend, game, fov, aspect, None)?;
                }
                for _ in 0..90 {
                    game.step(Intent::default(), tick);
                }
            }
        }
    }
    if options.hijack {
        if let Some(install) = install {
            hijack_for_screenshot(backend, game, fov, aspect, options, install)?;
        }
        return Ok(());
    }
    let entered = match &options.vehicle {
        Some(want) => {
            let here = game.focus();
            let world = game.scene.world.as_ref();
            let pick = game
                .scene
                .vehicles
                .iter()
                .enumerate()
                .filter(|(_, v)| v.spec.label.to_ascii_lowercase().contains(want.as_str()))
                .filter_map(|(i, v)| {
                    let at = world?
                        .physics
                        .vehicle_transform(v.id?)?
                        .transform_point3(m2_math::glam::Vec3::ZERO);
                    Some((i, at.distance(here)))
                })
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i);
            if let (Some(i), Some(p)) = (pick, game.scene.player.as_mut()) {
                p.driving = Some(i);
                game.take_from_driver(i);
            }
            pick.is_some()
        }
        None => game.enter_nearest(f32::INFINITY),
    };
    if !entered {
        eprintln!("drive: no vehicle to get in");
        return Ok(());
    }
    let label = game
        .scene
        .player
        .as_ref()
        .and_then(|p| p.driving)
        .map(|i| game.scene.vehicles[i].spec.label.clone())
        .unwrap_or_default();
    // Seated: wait for the clips, then a picture of her sitting still.
    let mut seats = match install {
        Some(install) if options.seated => {
            let mut h = hijack::Hijacks::start(install, None);
            let mut input = Input::default();
            let waited = Instant::now();
            while h.loading() && waited.elapsed() < Duration::from_secs(120) {
                h.tick(game, &mut input, tick);
                game.step(Intent::default(), tick);
            }
            for _ in 0..60 {
                h.tick(game, &mut input, tick);
                game.step(Intent::default(), tick);
                h.seat(game, tick);
            }
            if let Some(p) = &game.scene.player {
                eprintln!(
                    "seat: in the {label}: seated {} at {:.2?}",
                    p.seated,
                    p.character.position.to_array()
                );
            }
            render(backend, game, fov, aspect, None)?;
            if let Some(out) = &options.screenshot {
                let stem = out.file_stem().and_then(|s| s.to_str()).unwrap_or("drive");
                write_ppm(backend, &out.with_file_name(format!("{stem}-seated.ppm")))?;
            }
            Some(h)
        }
        _ => None,
    };
    let driven = |game: &Game| {
        let i = game.scene.player.as_ref()?.driving?;
        let v = &game.scene.vehicles[i];
        Some((v.id?, v.spec.hull))
    };
    // Parked close behind another (the Stryker in the military line):
    // back out first, as a player would, then drive on round it.
    if let (Some(world), Some((id, hull))) = (game.scene.world.as_ref(), driven(game))
        && autoplay::blocked_ahead(&world.physics, id, hull, 2.0)
    {
        eprintln!("drive: something within 2 m ahead: backing out for 2 s");
        let back = Intent {
            movement: m2_math::glam::Vec2::NEG_Y,
            ..Intent::default()
        };
        for _ in 0..120 {
            game.step(back, tick);
        }
        let brake = Intent {
            hold_jump: true,
            ..Intent::default()
        };
        for _ in 0..30 {
            game.step(brake, tick);
        }
    }
    // A helicopter climbs for three seconds first, then flies on.
    let flies = game
        .scene
        .player
        .as_ref()
        .and_then(|p| p.driving)
        .is_some_and(|i| game.scene.vehicles[i].spec.is_helicopter());
    if flies {
        let ground = game.driving().map(|(at, _)| at.y);
        let climb = Intent {
            hold_jump: true,
            ..Intent::default()
        };
        for _ in 0..180 {
            game.step(climb, tick);
        }
        for _ in 0..60 {
            game.step(Intent::default(), tick);
        }
        if let (Some(ground), Some((at, speed))) = (ground, game.driving()) {
            eprintln!(
                "drive: helicopter climbed {:.1} m in 3 s, hovering at {speed:.1} m/s",
                at.y - ground
            );
        }
    }
    let ploughing = options
        .plough
        .as_deref()
        .is_some_and(|kind| plough_at(game, kind));
    let from = game.driving().map(|(at, _)| at);
    // Who is about, to see they keep moving while she drives.
    let before: Vec<m2_math::glam::Vec3> = game
        .scene
        .people
        .npcs
        .iter()
        .map(|n| n.character.position)
        .collect();
    for i in 0..240 {
        let steer = match (game.scene.world.as_ref(), driven(game)) {
            (Some(w), Some((id, hull))) if !ploughing => {
                autoplay::steer_round(&w.physics, id, hull)
            }
            _ => 0.0,
        };
        let throttle = Intent {
            movement: m2_math::glam::Vec2::new(steer, 1.0),
            ..Intent::default()
        };
        game.step(throttle, tick);
        if let Some(h) = &mut seats {
            h.seat(game, tick);
        }
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    if let (Some(from), Some((to, speed))) = (from, game.driving()) {
        eprintln!(
            "drive: {label} from {from:.2} to {to:.2} ({:.1} m), {speed:.1} m/s, {} objects knocked loose",
            (to - from).length(),
            game.scene.world.as_ref().map_or(0, |w| w.knocked())
        );
        // Hardly moved, or stopped: what holds it.
        if ((to - from).length() < 8.0 || speed.abs() < 1.0)
            && let Some(world) = game.scene.world.as_ref()
            && let Some(v) = game
                .scene
                .player
                .as_ref()
                .and_then(|p| p.driving)
                .map(|i| &game.scene.vehicles[i])
            && let Some(id) = v.id
        {
            eprintln!(
                "drive: held: {}",
                autoplay::stuck_report(world, id, v.spec.hull, false)
            );
        }
    }
    if let Some(v) = game
        .scene
        .player
        .as_ref()
        .and_then(|p| p.driving)
        .map(|i| &game.scene.vehicles[i])
    {
        eprintln!(
            "drive: {} wheels, {} road wheels, tracked {}, wheels rolled {:.1} m, turret pivot {:?}, gun pivot {:?} ({} nodes, muzzle on it {})",
            v.spec.wheels.len(),
            v.spec.road_wheels.len(),
            v.spec.tracked,
            v.rolled,
            v.spec.pivot,
            v.spec.gun_pivot,
            v.spec.gun.iter().filter(|g| **g).count(),
            v.spec.muzzle_on_gun
        );
        let (lo, hi) = v.spec.hull;
        let stand = |w: &m2_engine::vehicle::WheelSpec| format!("{:.2}", w.centre);
        eprintln!(
            "drive: hull {lo:.2} to {hi:.2}, turret box {:.2?}, mass {:.0} kg, wheels at [{}]",
            v.spec.turret_box,
            v.spec.mass,
            v.spec
                .wheels
                .iter()
                .map(stand)
                .collect::<Vec<_>>()
                .join(", ")
        );
        let t = &v.spec.tuning;
        if v.spec.is_helicopter() {
            eprintln!(
                "drive: helicopter: flight {:?}, chase {:?}, {} rotors",
                t.flight,
                t.chase,
                v.spec.rotors.len()
            );
        }
        eprintln!(
            "drive: template tuning: top speed {:?} m/s, turret {:?}, camera {:?}, gun {:?}",
            t.max_speed,
            t.turret.map(|t| format!(
                "{:.0} deg/s, gun {:.0}..{:.0} deg at {:.0} deg/s",
                t.yaw_speed.to_degrees(),
                t.pitch.0.to_degrees(),
                t.pitch.1.to_degrees(),
                t.pitch_speed.to_degrees()
            )),
            t.camera,
            t.gun.map(|g| format!(
                "{:08x}: {:.0} a minute, {} rounds, {:.0} m/s (most {:.0}), gravity x{:.1}, flash {:?} s, bank {:08x?}",
                g.weapon, g.rate_of_fire, g.rounds, g.velocity, g.max_velocity, g.gravity_scale, g.flash, g.sound_bank
            ))
        );
        eprintln!(
            "drive: health {:?}, armour {:?}, blasts x{:?}, burns below {:?}, rounds x{:?}",
            t.health, t.armour, t.blast_scale, t.burns_below, t.rounds
        );
        let arm = game.scene.world.as_ref().and_then(|w| {
            w.index
                .template_db
                .armament_for(&v.spec.label, v.spec.model)
        });
        eprintln!(
            "drive: armament {:?}; armed {:?}; her seat {}",
            arm.map(|a| (&a.seats, a.mounts.len(), &a.couplings)),
            v.spec
                .armed
                .iter()
                .map(|m| (m.seat, m.priority, m.weapon.name.as_str(), &m.nodes))
                .collect::<Vec<_>>(),
            game.combat.seat
        );
        let templated = game
            .scene
            .world
            .as_ref()
            .and_then(|w| w.index.template_db.engine_for_model(v.spec.model))
            .is_some();
        if let Some(e) = game.engine() {
            eprintln!(
                "drive: engine bank {:08x} {} ({}), throttle {:.1}",
                e.bank,
                sound::bank_name(e.bank).unwrap_or("?"),
                if templated {
                    "its template's"
                } else {
                    "by kind"
                },
                e.throttle
            );
            // What it plays (read here: the picture has no sound).
            if let Some(install) = install {
                sound::load_engine(install, e.bank);
                let tank = game
                    .scene
                    .player
                    .as_ref()
                    .and_then(|p| p.driving)
                    .is_some_and(|i| game.scene.vehicles[i].spec.cannon());
                if tank {
                    sound::log_tank_gun(install);
                }
            }
        }
    }
    let npcs = &game.scene.people.npcs;
    let moved = npcs
        .iter()
        .zip(&before)
        .filter(|(n, b)| n.alive && n.character.position.distance(**b) > 0.5)
        .count();
    // What those who stayed put are doing.
    let mut still = std::collections::BTreeMap::<&str, usize>::new();
    for (n, b) in npcs.iter().zip(&before) {
        if !(n.alive && n.character.position.distance(*b) > 0.5) {
            *still.entry(n.doing()).or_default() += 1;
        }
    }
    eprintln!(
        "drive: {moved} of {} people moved more than 0.5 m during the drive; the rest: {}",
        npcs.len().min(before.len()),
        still
            .iter()
            .map(|(what, k)| format!("{k} {what}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if options.wreck {
        wreck_at_speed(game, tick);
    }
    if let Some(rounds) = options.shoot_up {
        shoot_up(game, rounds, tick);
    }
    if options.look != 0.0 || options.fire {
        // Stop, then swing the camera and watch the turret follow.
        let brake = Intent {
            hold_jump: true,
            ..Intent::default()
        };
        for _ in 0..60 {
            game.step(brake, tick);
        }
        let swing = 10;
        for i in 0..90 {
            // `--switch`: the weapon switch pressed as a player presses it
            // (Tab, the pad's right bumper), through the input and the
            // game's tick, to the vehicle's other gun (playtest 8).
            if options.switch && i == 0 {
                let mut input = input::Input::default();
                input.button(input::Action::SwitchWeapon, true);
                game.tick(&mut input, &config::CameraConfig::default(), false, tick);
            }
            let look = if i < swing {
                m2_math::glam::Vec2::new(options.look / swing as f32, 0.0)
            } else {
                m2_math::glam::Vec2::ZERO
            };
            let trigger = combat::Trigger {
                fire: options.fire && i == 89,
                // `--gunner`: to the gunner's seat first.
                seat: options.gunner && i == 0,
                ..combat::Trigger::default()
            };
            game.step(
                Intent {
                    look,
                    trigger,
                    ..brake
                },
                tick,
            );
            if i % 15 == 14
                && let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving)
            {
                let v = &game.scene.vehicles[i];
                // The hull's heading, for the turret's on the island.
                let heading =
                    v.id.zip(game.scene.world.as_ref())
                        .and_then(|(id, w)| w.physics.vehicle_transform(id))
                        .map_or(0.0, |at| {
                            let f = at.transform_vector3(m2_math::glam::Vec3::Z);
                            f.x.atan2(f.z)
                        });
                let island = (heading + v.turret).to_degrees().rem_euclid(360.0);
                eprintln!(
                    "drive: turret at {:.0} deg on the hull ({island:.0} deg on the island, hull {:.0}), gun raised {:.1} deg, camera yaw {:.0} deg, pitch {:.0} deg",
                    v.turret.to_degrees(),
                    heading.to_degrees().rem_euclid(360.0),
                    v.gun.to_degrees(),
                    game.scene
                        .player
                        .as_ref()
                        .map_or(0.0, |p| p.yaw.to_degrees().rem_euclid(360.0)),
                    game.scene
                        .player
                        .as_ref()
                        .map_or(0.0, |p| p.pitch.to_degrees())
                );
            }
        }
        if options.fire {
            describe_vehicle_fire(game);
        }
    }
    Ok(())
}

/// Where the gun of the vehicle she drives fired from: its muzzle on the
/// model, and where its flash is, on the model and in the world.
fn describe_vehicle_fire(game: &Game) {
    let Some(v) = game
        .scene
        .player
        .as_ref()
        .and_then(|p| p.driving)
        .map(|i| &game.scene.vehicles[i])
    else {
        return;
    };
    // The gun chosen, and what the HUD's gun box counts for it
    // ([`gamehud::HudState::of`]: the machine gun's while it is chosen
    // and has a `WeaponUI`).
    let mg = v
        .machine_gun()
        .filter(|_| game.combat.vehicle_gun == 1)
        .and_then(|(w, left)| Some((w.ui?, left, w.clip)));
    eprintln!(
        "drive: fire: {} chosen; gun box {}",
        if game.combat.vehicle_gun == 1 {
            "machine gun"
        } else {
            "main gun"
        },
        match (mg, v.rounds()) {
            (Some((_, left, of)), _) => format!("machine gun, {left} of {of} rounds"),
            (None, Some(left)) => format!("cannon, {left} shells"),
            _ => "none".to_owned(),
        }
    );
    let body =
        v.id.zip(game.scene.world.as_ref())
            .and_then(|(id, w)| w.physics.vehicle_transform(id));
    let flash = game
        .combat
        .effects
        .iter()
        .rev()
        .find_map(|(e, ..)| match e {
            combat::Effect::Flash { at, size, .. } => Some((*at, *size)),
            _ => None,
        });
    match (v.spec.muzzle, body, flash) {
        (None, ..) => eprintln!("drive: fire: no gun (no muzzle on a turret)"),
        (Some(m), Some(body), Some((at, size))) => eprintln!(
            "drive: fire: muzzle {m:.2} on the model, turret at {:.0} deg, gun raised {:.1} deg; flash {size:.2} m at {:.2} on the model, {at:.2} in the world; {} shells, {} rockets flying",
            v.turret.to_degrees(),
            v.gun.to_degrees(),
            body.inverse().transform_point3(at),
            game.combat.shells(),
            game.combat
                .projectiles()
                .filter(|(what, ..)| *what == combat::Flying::Rocket)
                .count()
        ),
        (Some(m), _, None) => eprintln!("drive: fire: muzzle {m:.2}, but no flash"),
        (Some(m), None, _) => eprintln!("drive: fire: muzzle {m:.2}, vehicle not placed"),
    }
}

/// Stops the vehicle she drives and puts `rounds` rifle rounds (9
/// damage, a `Car` taking a fortieth) into it, one part after another at
/// each part's middle, as the game's soldiers would; then logs what its
/// parts came to: how many changed state and look, those thrown off, the
/// effects running, and its health.
fn shoot_up(game: &mut Game, rounds: u32, tick: f32) {
    let brake = Intent {
        hold_jump: true,
        ..Intent::default()
    };
    for _ in 0..60 {
        game.step(brake, tick);
    }
    let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving) else {
        eprintln!("shoot-up: not driving");
        return;
    };
    let Some(body) = game.scene.vehicles[i]
        .id
        .and_then(|id| game.scene.world.as_ref()?.physics.vehicle_transform(id))
    else {
        return;
    };
    // Let its parts get their health from its template.
    game.step(brake, tick);
    let v = &mut game.scene.vehicles[i];
    let Some(dm) = v.spec.damage.clone() else {
        eprintln!("shoot-up: {} has no part states", v.spec.label);
        return;
    };
    let targets: Vec<m2_math::glam::Vec3> = (0..dm.boxes.len())
        .filter(|&k| Some(k) != v.parts.as_ref().and_then(|p| p.body))
        .filter_map(|k| dm.boxes[k].map(|(lo, hi)| (lo + hi) * 0.5))
        .collect();
    if targets.is_empty() {
        eprintln!("shoot-up: {} has no parts to hit", v.spec.label);
        return;
    }
    let start = v
        .parts
        .as_ref()
        .map(|p| p.state.clone())
        .unwrap_or_default();
    let looks_before = v.parts.as_ref().map(|p| p.hidden(&dm));
    let scale = v.spec.tuning.rounds.map_or(0.0, |r| r.bullet);
    for k in 0..rounds as usize {
        let at = body.transform_point3(targets[k % targets.len()]);
        v.hit_parts(body, at, 9.0);
        v.damage = (v.damage + 9.0 * scale).min(v.most_health() - 1.0);
    }
    // Five seconds: long enough for a car shot down to where it burns
    // to burn out.
    for _ in 0..300 {
        game.step(brake, tick);
    }
    let v = &game.scene.vehicles[i];
    let Some(p) = &v.parts else { return };
    let changed = p.state.iter().zip(&start).filter(|(a, b)| a != b).count();
    let looks_changed = looks_before.is_some_and(|h| h != p.hidden(&dm));
    eprintln!(
        "shoot-up: {} took {rounds} rounds over {} parts: {changed} of {} parts changed state, look changed {looks_changed}, {} thrown off lying, {} effects running, health {:.0}, wrecked {}",
        v.spec.label,
        targets.len(),
        dm.states.machines.len(),
        v.loose.len(),
        p.emitters(&dm).len(),
        v.health(),
        v.wrecked
    );
}

/// Wrecks the vehicle she drives, as it goes, and logs how it comes to
/// rest: how fast it went, how far it slid, and how much lower it sits.
fn wreck_at_speed(game: &mut Game, tick: f32) {
    let Some(i) = game.scene.player.as_ref().and_then(|p| p.driving) else {
        return;
    };
    let Some(id) = game.scene.vehicles[i].id else {
        return;
    };
    let state = |game: &Game| {
        let physics = &game.scene.world.as_ref()?.physics;
        Some((
            physics.vehicle_transform(id)?.w_axis.truncate(),
            physics.vehicle_velocity(id).length(),
        ))
    };
    let Some((from, speed)) = state(game) else {
        return;
    };
    game.scene.vehicles[i].wrecked = true;
    if let Some(world) = game.scene.world.as_mut() {
        world.physics.wreck_vehicle(id);
    }
    let mut stopped = None;
    for k in 0..240 {
        game.step(Intent::default(), tick);
        if stopped.is_none() && state(game).is_some_and(|(_, v)| v < 0.3) {
            stopped = Some(k as f32 * tick);
        }
    }
    if let Some((to, now)) = state(game) {
        eprintln!(
            "drive: wrecked at {speed:.1} m/s; slid {:.1} m, {:.2} m lower, moving {now:.2} m/s after 4 s, stopped {}",
            (to - from).with_y(0.0).length(),
            from.y - to.y,
            stopped.map_or("never".to_owned(), |t| format!("after {t:.1} s"))
        );
    }
}

/// Turns the player's camera so the crosshair is on `target`.
fn aim_at(game: &mut Game, target: m2_math::glam::Vec3) {
    if let Some(p) = game.scene.player.as_mut() {
        p.aim_crosshair_at(target);
    }
}

/// Fights for the screenshot: shoots the nearest soldier, fires a rocket
/// at the nearest parked vehicle and throws a grenade, printing how it
/// went, then takes the picture aiming at the soldier.
fn fight_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    wait: f32,
) -> anyhow::Result<()> {
    use combat::{Trigger, Weapon};
    let tick = TICK.as_secs_f32();
    let here = game.focus();
    let mut soldier = game
        .scene
        .people
        .npcs
        .iter()
        .enumerate()
        .filter(|(_, n)| n.soldier && n.alive && n.landed)
        .min_by(|a, b| {
            a.1.character
                .position
                .distance(here)
                .total_cmp(&b.1.character.position.distance(here))
        })
        .map(|(i, _)| i);
    let fire = |select| Intent {
        trigger: Trigger {
            fire: true,
            aim: true,
            select,
            ..Trigger::default()
        },
        ..Intent::default()
    };
    if let Some(i) = soldier {
        // Spawns are hundreds of metres out: start the fight 25 m away.
        let at = game.scene.people.npcs[i].character.position;
        if let Some(p) = game.scene.player.as_mut() {
            let away = (here - at)
                .with_y(0.0)
                .try_normalize()
                .unwrap_or(m2_math::glam::Vec3::X);
            p.character.position = at + away * 25.0 + m2_math::glam::Vec3::Y * 2.0;
            p.character.velocity = m2_math::glam::Vec3::ZERO;
            p.landed = false;
        }
        // People come and go as she moves, so find him again by spawn.
        let spawn = game.scene.people.npcs[i].spawn;
        for _ in 0..90 {
            game.step(Intent::default(), tick);
        }
        soldier = game
            .scene
            .people
            .npcs
            .iter()
            .position(|n| n.spawn.is_some() && n.spawn == spawn);
    }
    if let Some(i) = soldier {
        let here = game.focus();
        let at = game.scene.people.npcs[i].character.position;
        eprintln!(
            "fight: shooting the soldier at {at:.1} ({:.1} m)",
            at.distance(here)
        );
        aim_at(game, game.scene.people.npcs[i].chest());
        if let (Some(world), Some(player)) = (&game.scene.world, &game.scene.player) {
            let n = &game.scene.people.npcs[i];
            let (eye, dir) = (player.eye(), player.aim_forward());
            let to = n.chest() - eye;
            let miss = (to - dir * to.dot(dir)).length();
            let blocked = world.physics.cast_ray(eye, to, to.length()).map(|h| h.0);
            eprintln!(
                "fight: eye {eye:.1} dir {dir:.2} chest {:.1} ({:.1} m, ray passes {miss:.2} m off); \
                 alive {} landed {}; world between at {blocked:?}",
                n.chest(),
                to.length(),
                n.alive,
                n.landed,
            );
            let probe = combat::Combat::aim_probe(&world.physics, &game.scene.people, player);
            eprintln!(
                "fight: aim hits {}",
                match probe {
                    Some((d, Some(j))) => format!("person {j} at {d:.1} m (target {i})"),
                    Some((d, None)) => format!("the world at {d:.1} m"),
                    None => "nothing".to_owned(),
                }
            );
        }
        for _ in 0..90 {
            let chest = game.scene.people.npcs[i].chest();
            aim_at(game, chest);
            game.step(fire(Some(Weapon::Rifle)), tick);
        }
        let n = &game.scene.people.npcs[i];
        eprintln!("fight: soldier alive {}, hp {:.0}", n.alive, n.hp);
    } else {
        eprintln!("fight: no soldier near");
    }
    // A rocket at the nearest parked vehicle in range, else by the
    // soldier.
    let here = game.focus();
    let vehicle = game
        .scene
        .vehicles
        .iter()
        .filter_map(|v| {
            let id = v.id?;
            let world = game.scene.world.as_ref()?;
            Some(
                world
                    .physics
                    .vehicle_transform(id)?
                    .transform_point3(m2_math::glam::Vec3::Y),
            )
        })
        .filter(|at| at.distance(here) < 80.0)
        .min_by(|a, b| a.distance(here).total_cmp(&b.distance(here)))
        .or_else(|| {
            soldier.map(|i| {
                game.scene.people.npcs[i].character.position + m2_math::glam::Vec3::X * 2.0
            })
        });
    if let Some(at) = vehicle {
        aim_at(game, at);
        game.step(fire(Some(Weapon::Rpg)), tick);
        for _ in 0..120 {
            game.step(Intent::default(), tick);
        }
        eprintln!(
            "fight: rocket at {at:.1}; {} wrecked",
            game.scene.vehicles.iter().filter(|v| v.wrecked).count()
        );
    }
    game.step(
        Intent {
            trigger: Trigger {
                throw: true,
                select: Some(Weapon::Rifle),
                ..Trigger::default()
            },
            ..Intent::default()
        },
        tick,
    );
    for _ in 0..200 {
        game.step(Intent::default(), tick);
    }
    // Wrecks should stay put, not slide on.
    if let Some(world) = &game.scene.world {
        for v in game.scene.vehicles.iter().filter(|v| v.wrecked) {
            if let Some(id) = v.id {
                eprintln!(
                    "fight: wreck {} at {:.1}, moving {:.2} m/s",
                    v.spec.label,
                    world
                        .physics
                        .vehicle_transform(id)
                        .map_or(m2_math::glam::Vec3::ZERO, |t| t
                            .transform_point3(m2_math::glam::Vec3::ZERO)),
                    world.physics.vehicle_velocity(id).length()
                );
            }
        }
    }
    let c = &game.combat;
    let people = &game.scene.people.npcs;
    eprintln!(
        "fight: {} hits, {} blasts; player hp {:.0}{}; {} of {} people dead ({} limp), {} soldiers alert",
        c.hits,
        c.blasts,
        c.hp,
        if c.down > 0.0 { " (down)" } else { "" },
        people.iter().filter(|n| !n.alive).count(),
        people.len(),
        people.iter().filter(|n| n.ragdoll.is_some()).count(),
        people.iter().filter(|n| n.alert).count(),
    );
    // Smoke clears and bodies settle.
    for i in 0..(wait / tick) as usize {
        game.step(Intent::default(), tick);
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    // A body to look at: stand a few metres from it.
    let body = soldier.and_then(|i| {
        let doll = game.scene.people.npcs.get(i)?.ragdoll?;
        let nodes = game.scene.world.as_ref()?.physics.ragdoll_nodes(doll)?;
        let points: Vec<_> = nodes
            .iter()
            .map(|m| m.transform_point3(m2_math::glam::Vec3::ZERO))
            .collect();
        let (lo, hi) = points.iter().fold(
            (
                m2_math::glam::Vec3::splat(f32::MAX),
                m2_math::glam::Vec3::splat(f32::MIN),
            ),
            |(lo, hi), p| (lo.min(*p), hi.max(*p)),
        );
        eprintln!(
            "fight: body at {:.1}, {} nodes spanning {:.2}",
            (lo + hi) / 2.0,
            points.len(),
            hi - lo
        );
        Some((lo + hi) / 2.0)
    });
    // The picture: the body from close above, else aiming, with
    // soldiers firing back.
    if let (Some(at), Some(i)) = (body, soldier) {
        let n = &game.scene.people.npcs[i];
        eprintln!(
            "fight: body drawn: landed {}, {:.1} m from her",
            n.landed,
            n.character.position.distance(game.focus())
        );
        game.on_foot = false;
        game.camera = FlyCamera::looking_at(at + m2_math::glam::Vec3::new(1.5, 2.5, 1.5), at);
        render(backend, game, fov, aspect, None)?;
        return Ok(());
    } else if let Some(i) = soldier {
        let chest = game.scene.people.npcs[i].chest();
        aim_at(game, chest + m2_math::glam::Vec3::Y * 0.5);
    }
    for _ in 0..20 {
        game.step(
            Intent {
                trigger: Trigger {
                    aim: true,
                    ..Trigger::default()
                },
                ..Intent::default()
            },
            tick,
        );
    }
    // How far the aim clip alone points off the crosshair, and what is
    // left once the pose is turned to it.
    if let Some((_, rig)) = game.scene.player.as_ref().and_then(|p| p.model.as_ref())
        && let Some(want) = rig.aim
    {
        let pose = rig.animator.pose(&rig.skeleton, &rig.clips);
        let off = rig.aim_error(&pose, want);
        let left = rig.aimed_pose().and_then(|p| rig.aim_error(&p, want));
        let deg = |e: Option<(f32, f32)>| {
            e.map(|(y, p)| format!("yaw {:.0} pitch {:.0}", y.to_degrees(), p.to_degrees()))
        };
        eprintln!(
            "aim: clip off by {:?}, turned off by {:?}",
            deg(off),
            deg(left)
        );
    }
    render(backend, game, fov, aspect, None)?;
    Ok(())
}

/// Calls a Satellite Guided Bomb on the spot [`SATELLITE_AHEAD`] metres
/// ahead of her as a player would (the support key, the use key, the view
/// steered over it and confirmed, the wheel pressed in each wedge), for
/// the picture: the support menu just opened, looking through the
/// satellite, with the timing wheel's check after the first press, or
/// `wait` seconds (at least 0.8) after the bomb lands, from her camera.
fn satellite_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    stage: SatelliteShot,
    wait: f32,
) -> anyhow::Result<()> {
    use combat::Trigger;
    use m2_math::glam::{Vec2, Vec3};
    let tick = TICK.as_secs_f32();
    let Some(p) = game.scene.player.as_ref() else {
        return Ok(());
    };
    let her = p.character.position;
    let spot = her + Vec3::new(p.yaw.sin(), 0.0, p.yaw.cos()) * SATELLITE_AHEAD;
    let goal = Vec2::new(spot.x, spot.z);
    game.support.grant(support::Kind::SatelliteBomb, 1, Some(1));
    let press = |game: &mut Game, intent: Intent| {
        game.step(intent, tick);
        game.step(Intent::default(), tick);
    };
    let keys = |trigger: Trigger| Intent {
        trigger,
        ..Intent::default()
    };
    press(
        game,
        keys(Trigger {
            support: true,
            ..Trigger::default()
        }),
    );
    if stage == SatelliteShot::Menu {
        return Ok(());
    }
    press(
        game,
        Intent {
            use_pressed: true,
            ..Intent::default()
        },
    );
    // The designator comes up, and the view opens.
    for _ in 0..60 {
        if !game.support.holding() {
            break;
        }
        game.step(Intent::default(), tick);
    }
    for i in 0..1200 {
        let Some(at) = game.support.looking() else {
            break;
        };
        let to = goal - at;
        if to.length() < 0.3 {
            break;
        }
        // Screen right is the world's -x, looking down.
        let movement = Vec2::new(-to.x, to.y).clamp_length_max(1.0);
        game.step(
            Intent {
                movement,
                ..Intent::default()
            },
            tick,
        );
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    eprintln!(
        "satellite: {:?}, her at {her:.1}, the spot {goal:.1}",
        game.support.phase
    );
    if stage == SatelliteShot::View {
        return Ok(());
    }
    press(
        game,
        keys(Trigger {
            fire: true,
            ..Trigger::default()
        }),
    );
    // A press as the marker passes through each wedge; the wheel's
    // picture is of the marker in the first, before the press (as the
    // footage shows the wheel turning).
    for wedge in 0..2 {
        for _ in 0..600 {
            if game.support.wheel_ready() {
                break;
            }
            game.step(Intent::default(), tick);
        }
        if stage == SatelliteShot::Wheel {
            return Ok(());
        }
        press(
            game,
            Intent {
                use_pressed: true,
                ..Intent::default()
            },
        );
        eprintln!("satellite: wedge {wedge}: {:?}", game.support.phase);
    }
    let (hp, alive) = (
        game.combat.hp,
        game.scene.people.npcs.iter().filter(|n| n.alive).count(),
    );
    let mut landed = false;
    for i in 0..1200 {
        game.step(Intent::default(), tick);
        landed |= game
            .support
            .landed
            .iter()
            .any(|l| matches!(l, support::Impact::Bomb(_)));
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
        if landed {
            break;
        }
    }
    for i in 0..(wait.max(0.8) / tick) as usize {
        game.step(Intent::default(), tick);
        if i % 10 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    eprintln!(
        "satellite: the bomb {}; her health {hp:.0} -> {:.0}, people standing {alive} -> {}, \
         flash {:.2}, shaking {:.2} s",
        if landed { "landed" } else { "never landed" },
        game.combat.hp,
        game.scene.people.npcs.iter().filter(|n| n.alive).count(),
        game.support.flash,
        game.support.shake.1
    );
    Ok(())
}

/// Blows up the fuel tank or barrel nearest where she starts for the
/// screenshot: stands 25 m off, fires a rocket at it, and takes the
/// picture looking at where it stood.
fn blast_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    wait: f32,
) -> anyhow::Result<()> {
    use combat::{Trigger, Weapon};
    let tick = TICK.as_secs_f32();
    let here = game.focus();
    let Some(at) = game
        .scene
        .explosives
        .iter()
        .map(|e| e.at)
        .min_by(|a, b| a.distance(here).total_cmp(&b.distance(here)))
    else {
        eprintln!("blast: no explosives");
        return Ok(());
    };
    let near = |game: &Game, r: f32| {
        game.scene
            .explosives
            .iter()
            .filter(|e| e.at.distance(at) < r)
            .count()
    };
    eprintln!(
        "blast: explosive at {at:.1} ({:.0} m off); {} within 10 m, {} within 30 m",
        at.distance(here),
        near(game, 10.0),
        near(game, 30.0)
    );
    // Somewhere 20 m off with a clear line to it, trying round from
    // where she starts.
    let toward = (here - at).with_y(0.0);
    let start = toward.z.atan2(toward.x);
    let stand = (0..16)
        .map(|k| {
            let a = start + k as f32 * std::f32::consts::TAU / 16.0;
            at + m2_math::glam::Vec3::new(a.cos(), 0.0, a.sin()) * 20.0
        })
        .find(|&p| {
            let Some(world) = &game.scene.world else {
                return false;
            };
            let eye = p + m2_math::glam::Vec3::Y * 1.5;
            let to = at - eye;
            world
                .physics
                .cast_ray(eye, to, to.length())
                .is_none_or(|h| h.0 > to.length() - 6.0)
        });
    eprintln!("blast: standing at {stand:.1?}");
    if let Some(p) = game.scene.player.as_mut() {
        let toward = toward.try_normalize().unwrap_or(m2_math::glam::Vec3::X);
        p.character.position = stand.unwrap_or(at + toward * 20.0) + m2_math::glam::Vec3::Y * 2.0;
        p.character.velocity = m2_math::glam::Vec3::ZERO;
        p.landed = false;
    }
    for _ in 0..90 {
        game.step(Intent::default(), tick);
    }
    aim_at(game, at);
    if let (Some(world), Some(player)) = (&game.scene.world, &game.scene.player) {
        let (eye, dir) = (player.eye(), player.aim_forward());
        eprintln!(
            "blast: eye {eye:.1}, {:.1} m to it, aim ray hits at {:?}",
            eye.distance(at),
            world.physics.cast_ray(eye, dir, 100.0).map(|h| h.0)
        );
    }
    // Shoulder the launcher first, as a player would.
    let aim = |fire, select| Intent {
        trigger: Trigger {
            fire,
            aim: true,
            select,
            ..Trigger::default()
        },
        ..Intent::default()
    };
    game.step(aim(false, Some(Weapon::Rpg)), tick);
    for _ in 0..30 {
        aim_at(game, at);
        game.step(aim(false, None), tick);
    }
    game.step(aim(true, None), tick);
    eprintln!(
        "blast: rocket from {:.1?} towards {:.1?}",
        game.combat.last_launch.map(|l| l.0),
        game.combat.last_launch.map(|l| l.1)
    );
    // Until the rocket bursts (4 s at most), so the wait counts from it.
    let before = game.combat.blasts;
    for _ in 0..240 {
        game.step(Intent::default(), tick);
        if game.combat.blasts > before {
            break;
        }
    }
    let spent = game.scene.explosives.iter().filter(|e| e.gone).count();
    eprintln!(
        "blast: {} blasts, the last at {:.1?}; {spent} of {} explosives spent; player hp {:.0}",
        game.combat.blasts,
        game.combat.last_blast,
        game.scene.explosives.len(),
        game.combat.hp
    );
    if let Some(p) = &game.scene.particles {
        eprintln!(
            "blast: {} particles alive, {:.2} m from the blast on average",
            p.live(),
            p.mean_distance(at)
        );
    }
    for i in 0..(wait / tick) as usize {
        game.step(Intent::default(), tick);
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    aim_at(game, at);
    game.step(Intent::default(), tick);
    render(backend, game, fov, aspect, None)?;
    if let Some(p) = &game.scene.particles {
        eprintln!(
            "blast: {} particles alive at the picture, {:.2} m from the blast on average",
            p.live(),
            p.mean_distance(at)
        );
    }
    Ok(())
}

/// Rockets the nearest of the island's buildings (an object whose
/// template has health and a collapse, its model a wreck) until it comes
/// down, at most `rockets`, printing the damage it takes; pictures before,
/// and 1 s and 6 s after it comes down, beside `out`.
fn demolish_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    rockets: u32,
    out: &std::path::Path,
) -> anyhow::Result<()> {
    use combat::{Trigger, Weapon};
    use m2_math::glam::Vec3;
    let tick = TICK.as_secs_f32();
    let stem = out
        .file_stem()
        .map_or("shot".into(), |s| s.to_string_lossy());
    let picture = |name: &str| out.with_file_name(format!("{stem}-{name}.ppm"));
    // The island round her in first.
    for i in 0..600 {
        game.step(Intent::default(), tick);
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
        if i > 120
            && game
                .scene
                .world
                .as_ref()
                .is_some_and(|w| w.stats().pending == 0)
        {
            break;
        }
    }
    let here = game.focus();
    let Some(world) = &game.scene.world else {
        return Ok(());
    };
    let db = &world.index.template_db;
    let mut blasts: Vec<String> = db
        .explosions
        .iter()
        .filter_map(|(id, e)| {
            let n = db.names.get(id)?;
            Some(format!(
                "{n:?} {:.0} m {:.0}..{:.0}",
                e.radius, e.damage, e.least
            ))
        })
        .collect();
    blasts.sort();
    eprintln!("demolish: blasts: {}", blasts.join("; "));
    let target = world
        .near(here, 150.0)
        .into_iter()
        .filter(|&(i, _)| {
            let o = &world.index.objects[i];
            world.loose_mass(i).is_none()
                && world.object_model(i).is_some_and(|m| m.has_wreck())
                && db
                    .durability
                    .get(&o.template)
                    .is_some_and(|d| !d.invincible && d.health > 0.0 && d.collapse.is_some())
        })
        .min_by(|a, b| a.1.total_cmp(&b.1));
    let Some((i, _)) = target else {
        eprintln!("demolish: no building within 150 m of {here:.1}");
        return Ok(());
    };
    let o = world.index.objects[i];
    let (lo, hi) = world
        .object_model(i)
        .map_or((Vec3::ZERO, Vec3::ZERO), |m| m.bounds);
    let at = o.position + o.rotation * ((lo + hi) * 0.5);
    eprintln!(
        "demolish: object {i} model {:08x} template {:08x} {:?} at {:.1}, {:.1?} m, {:?}",
        o.model,
        o.template,
        db.names.get(&o.template),
        o.position,
        hi - lo,
        db.durability.get(&o.template)
    );
    // Stand 35 m off with a clear line to it, trying round from where she
    // starts.
    let toward = (here - at).with_y(0.0);
    let start = toward.z.atan2(toward.x);
    let far = 35.0 + (hi - lo).with_y(0.0).length() * 0.5;
    let stand = (0..16)
        .map(|k| {
            let a = start + k as f32 * std::f32::consts::TAU / 16.0;
            at.with_y(o.position.y) + Vec3::new(a.cos(), 0.0, a.sin()) * far
        })
        .find(|&p| {
            let eye = p + Vec3::Y * 1.5;
            let to = at - eye;
            world
                .physics
                .cast_ray(eye, to, to.length())
                .is_none_or(|h| h.0 > to.length() - (hi - lo).length() * 0.5)
        });
    // On the ground there, which needn't be as high as the building's foot.
    let stand = stand.unwrap_or(at.with_y(o.position.y) + toward.normalize_or(Vec3::X) * far);
    let stand = world
        .physics
        .island_below(stand.with_y(stand.y + 200.0), 400.0)
        .map_or(stand, |g| stand.with_y(g));
    eprintln!("demolish: standing at {stand:.1}");
    if let Some(p) = game.scene.player.as_mut() {
        p.character.position = stand + Vec3::Y * 2.0;
        p.character.velocity = Vec3::ZERO;
        p.landed = false;
    }
    for k in 0..120 {
        game.step(Intent::default(), tick);
        if k % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    let aim = |fire, select| Intent {
        trigger: Trigger {
            fire,
            aim: true,
            select,
            ..Trigger::default()
        },
        ..Intent::default()
    };
    aim_at(game, at);
    game.step(aim(false, Some(Weapon::Rpg)), tick);
    render(backend, game, fov, aspect, None)?;
    write_ppm(backend, &picture("before"))?;
    eprintln!(
        "demolish: player at {:.1?}, landed {:?}, on foot {}, weapon {:?}",
        game.scene.player.as_ref().map(|p| p.character.position),
        game.scene.player.as_ref().map(|p| p.landed),
        game.on_foot,
        game.combat.weapon
    );
    let mut fired = 0;
    while fired < rockets && !game.destruction.destroyed.contains(&i) {
        for _ in 0..30 {
            aim_at(game, at);
            game.step(aim(false, None), tick);
        }
        game.step(aim(true, None), tick);
        fired += 1;
        for _ in 0..120 {
            aim_at(game, at);
            game.step(aim(false, None), tick);
        }
        eprintln!(
            "demolish: rocket {fired} from {:.1?}, {} blasts, the last at {:.1?}; damage {:.0}",
            game.combat.last_launch.map(|l| l.0),
            game.combat.blasts,
            game.combat.last_blast,
            game.destruction.damage(i)
        );
    }
    // No rocket left her (she hasn't landed where she stands): blasts at
    // its middle as a rocket's would be, to see it come down all the same.
    if game.combat.last_launch.is_none()
        && let (Some(world), Some(install)) =
            (game.scene.world.as_mut(), game.scene.install.as_ref())
    {
        for k in 0..rockets {
            if game.destruction.destroyed.contains(&i) {
                break;
            }
            for blast in crate::game::rocket_blasts(&world.index.template_db, at) {
                game.destruction
                    .blast(world, &mut game.scene.props, Some(install), blast);
            }
            eprintln!(
                "demolish: no rocket left her; direct blast {} at {at:.1}: damage {:.0}",
                k + 1,
                game.destruction.damage(i)
            );
        }
    }
    if !game.destruction.destroyed.contains(&i) {
        eprintln!("demolish: still standing after {fired} rockets");
        return Ok(());
    }
    for (name, wait) in [("1s", 1.0), ("2s", 1.0), ("6s", 4.0)] {
        for k in 0..(wait / tick) as usize {
            aim_at(game, at);
            game.step(Intent::default(), tick);
            if k % 30 == 0 {
                render(backend, game, fov, aspect, None)?;
            }
        }
        render(backend, game, fov, aspect, None)?;
        write_ppm(backend, &picture(name))?;
        eprintln!(
            "demolish: {name}: taken {}, {}; player at {:.1?}",
            game.scene.world.as_ref().is_some_and(|w| w.is_taken(i)),
            game.scene.props.counts(),
            game.scene.player.as_ref().map(|p| p.character.position)
        );
        if let Some(world) = &game.scene.world
            && let Some(report) = game
                .scene
                .props
                .pieces_report(&world.physics, destruct::key(i))
        {
            eprintln!("demolish: {name}: pieces {report}");
        }
        if let Some(world) = &game.scene.world
            && let Some(s) = game.scene.props.describe(&world.physics, destruct::key(i))
        {
            eprintln!("demolish: {name}: {s}");
        }
    }
    Ok(())
}

/// Seconds between the frame-rate lines play logs ([`Pace`]).
const PACE_EVERY: f32 = 30.0;

/// The frame rate in play, logged every [`PACE_EVERY`] seconds as a
/// `pace:` line: frames a second, the slowest frame, and how many frames
/// took over 50 ms (under 20 fps), so a playtest's log says where play
/// slowed, which a title bar's count can't (playtest 5's report of a
/// lower frame rate had no numbers behind it).
#[derive(Debug, Default)]
struct Pace {
    /// When the current span started, and the last frame.
    since: Option<(Instant, Instant)>,
    frames: u32,
    slowest: Duration,
    slow: u32,
}

impl Pace {
    /// A frame drawn at `now` in a window `size` pixels: the line to log
    /// when a span is over, and its seconds.
    fn frame(&mut self, now: Instant, size: (u32, u32)) -> Option<(String, f32)> {
        let Some((start, last)) = self.since else {
            self.since = Some((now, now));
            // What was timed before play (loading) is not the span's.
            let _ = spans::take();
            return None;
        };
        let took = now - last;
        self.frames += 1;
        self.slowest = self.slowest.max(took);
        if took > Duration::from_millis(50) {
            self.slow += 1;
        }
        self.since = Some((start, now));
        let span = (now - start).as_secs_f32();
        if span < PACE_EVERY {
            return None;
        }
        let line = format!(
            "pace: {:.0} fps over {span:.0} s, slowest frame {:.0} ms, {} over 50 ms, {}x{}",
            self.frames as f32 / span,
            self.slowest.as_secs_f32() * 1000.0,
            self.slow,
            size.0,
            size.1
        );
        *self = Self {
            since: Some((now, now)),
            ..Self::default()
        };
        Some((line, span))
    }
}

struct Running {
    window: Arc<Window>,
    backend: WgpuBackend,
    game: Game,
    audio: sound::GameAudio,
    /// Online co-op, when hosting or joining.
    coop: Option<coop::Coop>,
    /// Whether she was driving last tick (getting in is worth a word).
    was_driving: bool,
    input: Input,
    /// The Xbox pad, read each frame into `input`.
    pad: pad::Pad,
    /// The mouse is captured and its pointer hidden: in play, as the
    /// game's (the pointer shows only in the front end and the pause
    /// menu), unless the mouse key let it go (`released`) or the window
    /// lost the focus.
    mouse_look: bool,
    released: bool,
    /// Keyboard input is ignored while another window has focus.
    focused: bool,
    /// Alt is held (Alt+Enter toggles fullscreen).
    alt: bool,
    /// What F11 and Alt+Enter toggle between.
    display: display::Display,
    /// The pause menu's keyboard table waits for a key or mouse button:
    /// the next one pressed goes to it, not to play.
    capturing: bool,
    culling: bool,
    /// In the background (Android): no surface to draw to, and play waits.
    background: bool,
    last: Instant,
    lag: Duration,
    fps: (Instant, u32),
    /// The frame rate as logged ([`Pace`]).
    pace: Pace,
    title: String,
    hud: ui::Hud,
    egui: egui_winit::State,
    mission: Option<mission::MissionLink>,
    hijacks: Option<hijack::Hijacks>,
    /// The launch's movies and front end (`--alpha`).
    flow: Option<flow::Flow>,
    /// The GUI's named buttons pressed since the last frame (Enter is
    /// `"selection"`), for the game HUD's scripts.
    buttons: Vec<String>,
    /// The pad buttons pressed for a dialog box the scripts have up
    /// ([`dialog_button`]), for the game HUD's.
    dialog_keys: Vec<&'static str>,
    /// The developer's keys work ([`App::dev`]).
    dev: bool,
    /// `--grapple`: the grappling hook without the scripts' leave.
    grapple: bool,
    /// The cheat menu's god mode and infinite ammo ([`cheats::keep_up`]).
    cheats: cheats::Toggles,
    /// The letters typed in play, for the cheat menu's code ([`cheats::CODE`]).
    cheat_code: cheats::CodeWatch,
    /// The code was typed: the cheat menu comes up on the next frame.
    cheat_typed: bool,
    /// A load asked to start the scripts again from this save: the alpha
    /// starts afresh with it ([`relaunch`]).
    relaunch: Option<std::path::PathBuf>,
    /// The pause menu's Quit: the alpha starts again at its front end
    /// ([`relaunch_front_end`]).
    to_front_end: bool,
    /// A menu is up and the world's sounds are held ([`m2_engine::Mixer::hold`]).
    sound_held: bool,
    /// When to load the player's own save as the pause menu does (`--load-at`):
    /// a test of loading in play.
    load_at: Option<Instant>,
    /// When to open the pause menu as Escape does (`--pause-at`): a test of
    /// pausing in play (the pause music).
    pause_at: Option<Instant>,
}

struct App {
    config: Config,
    install: Option<Install>,
    mode: Mode,
    /// Start with back-face culling off.
    double_sided: bool,
    /// Start flying rather than on foot.
    fly: bool,
    /// The developer's keys work: the free camera, quick save and load,
    /// back-face culling, the gait cycle. Off in the whole game's launch
    /// (`--alpha`, or no options) unless `--dev` asks: there the free
    /// camera would leave her body wherever the scripts lose track of
    /// it, and a quick load puts the world back without the scripts'
    /// state.
    dev: bool,
    /// `--grapple`: the grappling hook without the scripts' leave.
    grapple: bool,
    /// Run the mission scripts.
    mission: bool,
    /// Lua to run before they start (`--mission-driver`).
    mission_drivers: Vec<(String, String)>,
    /// Online co-op to host or join (`--host`, `--join`).
    coop: Option<coop::Setup>,
    /// Our stand-in HUD instead of the game's own.
    old_hud: bool,
    hijack_crew: Option<String>,
    /// Support items to call in (`--support`).
    support: Vec<String>,
    /// The whole game's launch (`--alpha`): boot movies (unless
    /// `no_boot`), front end, the scripts' movies.
    alpha: bool,
    no_boot: bool,
    /// Start at the front end, without the boot movies (`--to-front-end`).
    to_front_end: bool,
    /// End the scripts' skip mode after their start (unless
    /// `--skip-mode-on`).
    skip_mode_off: bool,
    /// The story skipped to this contract (`--skip-to`).
    skip_to: Option<(String, bool)>,
    /// Weapons to start with (`--gun`).
    guns: Vec<String>,
    /// A save to start from (`--load`).
    load: Option<save::SaveGame>,
    /// Seconds into play to load the player's own save (`--load-at`).
    load_at: Option<f32>,
    pause_at: Option<f32>,
    /// The alpha started again (the pause menu's Quit, loading a save),
    /// which this one waits for once its window is closed.
    successor: Option<std::process::Child>,
    state: Option<Running>,
    error: Option<anyhow::Error>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<Running> {
        let w = &self.config.window;
        let monitor = event_loop.primary_monitor();
        // Windowed, no larger than the monitor; fullscreen as the
        // settings keep it ([`display`]).
        let (full, how) =
            display::fullscreen_for(monitor.clone(), w.mode, (w.width, w.height), w.refresh);
        let attributes = Window::default_attributes()
            .with_title("Mercenaries 2 (alpha)")
            .with_inner_size(display::fit((w.width, w.height), monitor.as_ref()))
            .with_fullscreen(full);
        let window = Arc::new(event_loop.create_window(attributes)?);
        let display = display::Display::new(w);
        eprintln!("display: start {how}: {}", display::describe(&window));
        let size = window.inner_size();
        let mut backend =
            WgpuBackend::for_window(window.clone(), size.width, size.height, w.vsync)?;
        let adapter = backend.adapter_summary();
        eprintln!("adapter: {adapter:?}");
        backend.set_render_scale(w.render_scale);
        backend.set_light_shadows(cfg!(target_os = "android"));
        if w.render_scale < 1.0 {
            eprintln!(
                "render scale: the world drawn at {:.2} of the window",
                w.render_scale
            );
        }
        let mut game = build_scene(
            &mut backend,
            self.install.as_ref(),
            &self.mode,
            self.double_sided,
            self.config.world.sea_level,
            self.mission,
            self.config.hero,
        )?;
        game.scene.people.smarter = self.config.enhanced.smarter_ai;
        give_guns(&mut game, &self.guns);
        if self.fly {
            game.on_foot = false;
        }
        let focused = window.has_focus();
        let mut hud = ui::Hud::new();
        hud.game = game_hud(self.install.as_ref(), &self.mode, self.old_hud, &game);
        hud.loading = loading_screen(self.install.as_ref());
        hud.dev = self.dev;
        eprintln!(
            "keys: the developer's (F4 free camera, F5/F9 quick save and load, F3 culling, \
             N gait, F7 cheat menu) {}",
            if self.dev { "on" } else { "off (--dev)" }
        );
        let egui = egui_winit::State::new(
            hud.ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        let mut running = Running {
            title: format!(
                "Mercenaries 2 (alpha) - {} on {}",
                adapter.name, adapter.backend
            ),
            window,
            display,
            backend,
            game,
            audio: {
                let mut audio = sound::GameAudio::start(self.install.as_ref());
                audio.hero = self.config.hero;
                audio
            },
            coop: self
                .coop
                .as_ref()
                .map(|s| coop::Coop::start(s, self.config.hero)),
            was_driving: false,
            input: Input::default(),
            pad: {
                let mut pad = pad::Pad::default();
                pad.touch.resize(size.width, size.height);
                pad
            },
            mouse_look: false,
            released: false,
            focused,
            alt: false,
            capturing: false,
            culling: !self.double_sided,
            background: false,
            last: Instant::now(),
            lag: Duration::ZERO,
            fps: (Instant::now(), 0),
            pace: Pace::default(),
            egui,
            hud,
            mission: match (&self.install, &self.mode) {
                (Some(install), Mode::Island(_)) if self.mission => Some(
                    mission::MissionLink::start(install, self.mission_drivers.clone())
                        .with_save(self.load.take())
                        .with_keys(self.config.keys.clone())
                        .with_support(self.support.clone())
                        .with_movies(self.alpha)
                        .with_skip_mode_off(self.skip_mode_off)
                        .with_skip_to(self.skip_to.clone())
                        .with_subtitles(self.config.options.subtitles),
                ),
                _ => None,
            },
            hijacks: match (&self.install, &self.mode) {
                (Some(install), Mode::Island(_)) => {
                    Some(hijack::Hijacks::start(install, self.hijack_crew.clone()))
                }
                _ => None,
            },
            flow: match &self.install {
                Some(install) if self.alpha => Some(flow::Flow::new(
                    install.clone(),
                    !self.no_boot || self.to_front_end,
                    !self.to_front_end,
                    self.config.hero,
                    self.config.options.subtitles,
                )),
                _ => None,
            },
            buttons: Vec::new(),
            dialog_keys: Vec::new(),
            dev: self.dev,
            grapple: self.grapple,
            cheats: cheats::Toggles::default(),
            cheat_code: cheats::CodeWatch::default(),
            cheat_typed: false,
            relaunch: None,
            to_front_end: false,
            sound_held: false,
            load_at: self
                .load_at
                .map(|s| Instant::now() + Duration::from_secs_f32(s)),
            pause_at: self
                .pause_at
                .map(|s| Instant::now() + Duration::from_secs_f32(s)),
        };
        running.apply_options(&self.config);
        Ok(running)
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, e: anyhow::Error) {
        eprintln!("error: {e:#}");
        self.error = Some(e);
        event_loop.exit();
    }
}

impl Running {
    fn set_mouse_look(&mut self, on: bool) {
        // winit has no grab on Android: the activity captures the pointer.
        #[cfg(target_os = "android")]
        let grab = {
            android::capture_pointer(on);
            Ok::<(), ()>(())
        };
        #[cfg(not(target_os = "android"))]
        let grab = if on {
            self.window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| self.window.set_cursor_grab(CursorGrabMode::Confined))
        } else {
            self.window.set_cursor_grab(CursorGrabMode::None)
        };
        if grab.is_ok() {
            self.window.set_cursor_visible(!on);
            self.mouse_look = on;
        }
    }

    /// Captures the mouse and hides its pointer in play, and lets it go
    /// in the front end and the pause menu, as the game does; the pointer
    /// was left showing over play until a click captured it (round 2 of
    /// the owner's play tests). Each change is logged.
    fn keep_mouse(&mut self) {
        let menus = self.hud.menu || self.flow.as_ref().is_some_and(flow::Flow::in_menus);
        let want = !menus && self.focused && !self.released;
        if want != self.mouse_look {
            self.set_mouse_look(want);
            if self.mouse_look != want {
                // The window wouldn't grab it: not again until a click.
                eprintln!("mouse: the window would not capture it");
                self.released = true;
                return;
            }
            eprintln!(
                "mouse: {} ({})",
                if self.mouse_look {
                    "captured, pointer hidden"
                } else {
                    "free, pointer shown"
                },
                if menus {
                    "menus"
                } else if !self.focused {
                    "window not focused"
                } else if self.released {
                    "let go by its key"
                } else {
                    "play"
                }
            );
        }
    }

    /// The UI's output to the window, the pointer hidden while the mouse
    /// is captured: egui shows it again whenever its own pointer icon
    /// changes, as it does on the window's first frames and each time the
    /// pointer comes back in.
    fn platform_output(&mut self, mut output: egui::PlatformOutput) {
        if self.mouse_look {
            output.cursor_icon = egui::CursorIcon::None;
        }
        self.egui.handle_platform_output(&self.window, output);
    }

    /// Advances the simulation by whole ticks (none while the menu is
    /// open) and draws a frame with the HUD.
    fn frame(&mut self, config: &mut Config, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        if self.background {
            return Ok(());
        }
        let now = Instant::now();
        let dt = (now - self.last).min(MAX_FRAME).as_secs_f64();
        self.read_pad(dt);
        // The look keys (I J K L) turn the view as the pad's stick does.
        if self.focused && !self.hud.menu {
            self.input.key_look(dt);
        }
        self.lag += (now - self.last).min(MAX_FRAME);
        self.last = now;
        while self.lag >= TICK {
            self.lag -= TICK;
            spans::time("ticks", || self.tick(config, event_loop));
        }
        self.keep_mouse();
        let vsync = config.window.vsync;
        let input = self.egui.take_egui_input(&self.window);
        if let Some(flow) = self.flow.as_mut().filter(|f| f.covers()) {
            let output = self.hud.build_screen(input, |ui| flow.paint(ui));
            self.platform_output(output);
            let (w, h) = self.backend.size();
            return render(
                &mut self.backend,
                &mut self.game,
                config.camera.fov,
                w as f32 / h as f32,
                Some(&mut self.hud),
            );
        }
        let (w, h) = self.backend.size();
        let mut info = hud_info(
            &self.game,
            self.mission.as_ref(),
            config.camera.fov,
            w as f32 / h as f32,
        );
        if let Some(v) = self.hijacks.as_ref().and_then(hijack::Hijacks::view) {
            // The game's own prompt (`Hijack %s`); the buttons it asks
            // for are the HUD's to draw.
            info.prompt = (!v.running).then(|| format!("Hijack {}", v.vehicle));
            // With the keyboard, which keys the arrow means (playtest 12:
            // no tap of 9 rounds reached the scripts).
            if v.running && !self.input.pad() {
                info.hijack_keys = v
                    .prompts
                    .first()
                    .and_then(|p| hijack::key_hint(p, &config.keys));
            }
            info.can_enter = None;
        }
        // The hijack scripts' calls to the HUD, taken whether it runs or
        // not.
        let gui_calls = self
            .hijacks
            .as_ref()
            .map(hijack::Hijacks::take_gui_calls)
            .unwrap_or_default();
        let buttons = std::mem::take(&mut self.buttons);
        let selected = buttons.iter().any(|b| b == "selection");
        if self.hud.game.is_some() {
            let mut state = gamehud::HudState::of(&mut self.game);
            state.buttons = buttons;
            state.dialog_keys = std::mem::take(&mut self.dialog_keys);
            state.medevac = self
                .mission
                .as_ref()
                .and_then(mission::MissionLink::medevac_state);
            state.gui_calls = gui_calls;
            state.pad = self.input.pad();
            state.stick = self.input.stick();
            info.game = Some(state);
        }
        if self.pad.touch.used {
            info.touch = Some(self.pad.touch.picture());
        }
        let (action, output) = spans::time("hud", || self.hud.build(input, &info, config));
        self.platform_output(output);
        if let Some(news) = self
            .hud
            .game
            .as_mut()
            .map(gamehud::GameHud::take_pause_news)
        {
            self.pause_news(news, config, event_loop);
        }
        // The mission's callbacks the game's HUD called (the contract
        // fanfare closing), back to the mission's scripts.
        if let (Some(g), Some(m)) = (&mut self.hud.game, &mut self.mission) {
            for seq in g.take_callbacks() {
                m.hud_callback(seq);
            }
            // And the options picked in its dialog boxes.
            for (seq, picked) in g.take_choices() {
                m.hud_choice(seq, picked);
            }
        }
        // With no game HUD to show the contract's fanfare (`--old-hud`,
        // its scripts not up or broken, or the fanfare's call failed in
        // them), Enter closes it as the fanfare's own Enter does, and the
        // flow goes on to the next contract.
        let no_fanfare = self
            .hud
            .game
            .as_ref()
            .is_none_or(|g| !g.ready() || g.dropped_fanfare());
        if let Some(m) = &mut self.mission
            && selected
            && no_fanfare
            && !self.hud.menu
            && m.completed()
            && let Some(seq) = m.waiting_fanfare()
        {
            eprintln!(
                "mission: Enter closes the contract's fanfare (HUD call {seq}), which the \
                 game's HUD can't show"
            );
            m.hud_callback(seq);
        }
        if config.window.vsync != vsync {
            self.backend.set_vsync(config.window.vsync);
        }
        match action {
            Some(ui::MenuAction::Resume) => self.close_menu(),
            Some(ui::MenuAction::SaveSettings) => {
                let path = config::Config::default_path();
                match std::fs::write(&path, config.to_toml()) {
                    Ok(()) => eprintln!("settings saved to {}", path.display()),
                    Err(e) => eprintln!("could not save {}: {e}", path.display()),
                }
            }
            Some(ui::MenuAction::SaveGame) => self.save_game(),
            Some(ui::MenuAction::LoadGame) => {
                self.load_game();
                self.close_menu();
            }
            Some(ui::MenuAction::Hero(hero)) => {
                eprintln!("hero: playing as {hero}");
                self.set_hero(hero);
            }
            Some(ui::MenuAction::Display(mode)) => {
                let how = self.display.set(&self.window, mode, &mut config.window);
                eprintln!("pause: display: {how}");
                config.save("pause");
            }
            Some(ui::MenuAction::Quit) => event_loop.exit(),
            Some(ui::MenuAction::Shop(a)) => self.shop_action(a, config),
            Some(ui::MenuAction::Cheat(a)) => self.cheat_action(a),
            None => {}
        }
        let (w, h) = self.backend.size();
        render(
            &mut self.backend,
            &mut self.game,
            config.camera.fov,
            view_aspect(&config.window, w, h),
            Some(&mut self.hud),
        )?;
        self.fps.1 += 1;
        let elapsed = now - self.fps.0;
        if elapsed >= Duration::from_secs(1) {
            let fps = f64::from(self.fps.1) / elapsed.as_secs_f64();
            let p = self.game.focus();
            self.window.set_title(&format!(
                "{} - {fps:.0} fps - ({:.0}, {:.0}, {:.0})",
                self.title, p.x, p.y, p.z
            ));
            self.fps = (now, 0);
        }
        if let Some((line, span)) = self.pace.frame(now, (w, h)) {
            let s = &self.game.scene;
            let p = self.game.focus();
            eprintln!(
                "{line}; at ({:.0}, {:.0}), {} vehicles, {} people, {} riders; ms a second: {}",
                p.x,
                p.z,
                s.vehicles.len(),
                s.people.npcs.len(),
                s.riders.len(),
                spans::describe(&spans::take(), span, 12)
            );
        }
        Ok(())
    }

    /// Reads the Xbox pad into the input (`dt` seconds since the last
    /// read), and hands its menu keys on: to the front end while it holds
    /// the screen, else as keys to the menus and the HUD's movies (the
    /// PDA, a fanfare), A being Enter.
    fn read_pad(&mut self, dt: f64) {
        if !self.focused {
            return;
        }
        let driving = self.game.driving().is_some();
        let rumble = std::mem::take(&mut self.game.rumble);
        self.pad.rumble(rumble);
        let keys = self.pad.poll(&mut self.input, driving, dt);
        for key in keys {
            use pad::MenuKey;
            if let Some(flow) = self.flow.as_mut().filter(|f| f.holds()) {
                flow.key(match key {
                    MenuKey::Enter => KeyCode::Enter,
                    MenuKey::Escape => KeyCode::Escape,
                    MenuKey::Up => KeyCode::ArrowUp,
                    MenuKey::Down => KeyCode::ArrowDown,
                    MenuKey::Left => KeyCode::ArrowLeft,
                    MenuKey::Right => KeyCode::ArrowRight,
                });
                continue;
            }
            // A dialog box the scripts put up takes the pad's d-pad, A
            // and B ([`dialog_button`]).
            if !self.hud.menu
                && self
                    .mission
                    .as_ref()
                    .is_some_and(mission::MissionLink::dialog_open)
            {
                let b = match key {
                    MenuKey::Up => Some("BUTTON_PAD1_U"),
                    MenuKey::Down => Some("BUTTON_PAD1_D"),
                    MenuKey::Enter => Some("BUTTON_PAD2_D"),
                    MenuKey::Escape => Some("BUTTON_PAD2_R"),
                    MenuKey::Left | MenuKey::Right => None,
                };
                self.dialog_keys.extend(b);
                continue;
            }
            // Our pause menu's buttons are egui's: up and down step its
            // focus as Tab does (egui's arrows need one focused first).
            // The game's own takes the arrows (its movie's keys: up and
            // down through its list, left and right a setting, Enter);
            // B leaves it as Escape does, as Start would.
            let own = self.hud.menu
                && self.hud.shop.is_none()
                && self
                    .hud
                    .game
                    .as_ref()
                    .is_some_and(gamehud::GameHud::pause_available);
            let menu = self.hud.menu && !own;
            let (egui_key, modifiers) = match key {
                MenuKey::Enter => (egui::Key::Enter, egui::Modifiers::NONE),
                MenuKey::Escape => {
                    // B leaves the pause menu as Escape does (in play, B
                    // is the bash).
                    if self.hud.menu {
                        self.close_menu();
                    }
                    continue;
                }
                MenuKey::Up if menu => (egui::Key::Tab, egui::Modifiers::SHIFT),
                MenuKey::Down if menu => (egui::Key::Tab, egui::Modifiers::NONE),
                MenuKey::Up => (egui::Key::ArrowUp, egui::Modifiers::NONE),
                MenuKey::Down => (egui::Key::ArrowDown, egui::Modifiers::NONE),
                MenuKey::Left => (egui::Key::ArrowLeft, egui::Modifiers::NONE),
                MenuKey::Right => (egui::Key::ArrowRight, egui::Modifiers::NONE),
            };
            if key == MenuKey::Enter && !self.hud.menu {
                self.buttons.push("selection".into());
            }
            let events = &mut self.egui.egui_input_mut().events;
            for pressed in [true, false] {
                events.push(egui::Event::Key {
                    key: egui_key,
                    physical_key: None,
                    pressed,
                    repeat: false,
                    modifiers,
                });
            }
        }
    }

    fn open_menu(&mut self) {
        self.hud.menu = true;
        self.keep_mouse();
        self.input.clear();
    }

    fn close_menu(&mut self) {
        self.hud.cheats = None;
        if self.hud.shop.take().is_some()
            && let Some(m) = &mut self.mission
        {
            m.close_shop();
        }
        self.hud.menu = false;
        self.capturing = false;
        self.input.clear();
        self.released = false;
        self.keep_mouse();
    }

    /// The player's options as the game's pause menu shows them, with the
    /// window as it is and the saves there are.
    fn pause_settings(&self, config: &Config) -> pause::Settings {
        pause::Settings {
            profile: self.game.scene.hero.full_name().into(),
            video: display::current(&self.window),
            modes: display::modes(self.window.current_monitor().as_ref()),
            saves: pause_saves(self.game.scene.hero),
            ..config.menu_settings()
        }
    }

    /// The window as a video screen kept it (`menu`'s: the pause menu's
    /// or the front end's), into the settings: a size, or fullscreen at
    /// a mode.
    fn keep_video(window: &Window, menu: &str, v: pause::Video, config: &mut Config) {
        if v == display::current(window) {
            return;
        }
        let how = display::apply(window, v, &mut config.window);
        eprintln!(
            "{menu}: video kept: {}, {}: {how}",
            v.resolution_text(),
            v.refresh_text()
        );
    }

    /// Applies the player's options that live outside the settings file's
    /// readers: the volumes, the pad, the scripts' subtitles and tips.
    fn apply_options(&mut self, config: &Config) {
        let o = &config.options;
        self.set_volumes([o.sfx_volume, o.music_volume, o.dialog_volume]);
        self.pad.look = Some(pause::pad_scale(o.pad_sensitivity));
        self.pad.invert = o.invert_pad;
        self.pad.rumble_on = o.rumble;
        // The advanced video screen's fourth option is shadows.
        self.game.picture = game::Picture {
            brightness: o.brightness,
            shadows: o.advanced_video[3] != 0.0,
        };
        self.game.set_view_distance(o.view_distance);
        if let Some(m) = &mut self.mission {
            m.set_options(o.subtitles, o.tutorials);
        }
    }

    /// The SFX, music and dialog volumes, 0 to 100 (see
    /// [`pause::Settings`] for what each covers).
    fn set_volumes(&mut self, [sfx, music, dialog]: [f64; 3]) {
        use m2_engine::Category;
        let level = |v: f64| (v / 100.0) as f32;
        self.audio.out.with(|m| {
            for c in [Category::World, Category::Chatter, Category::Ui] {
                m.set_volume(c, level(sfx));
            }
            m.set_volume(Category::Music, level(music));
            m.set_volume(Category::Vo, level(dialog));
        });
    }

    /// Does what the game's pause menu asked: leaving it, saving, its
    /// settings kept (written to the settings file, as the game saves its
    /// profile), its sounds.
    fn pause_news(&mut self, news: pause::News, config: &mut Config, event_loop: &ActiveEventLoop) {
        for cue in &news.stops {
            self.audio.stop_ui_sound(cue);
        }
        for cue in &news.sounds {
            self.audio.ui_sound(cue);
        }
        if let Some(v) = news.volumes {
            self.set_volumes(v);
        }
        if news.medevac
            && let Some(m) = &mut self.mission
        {
            m.medevac();
        }
        if let Some(cue) = news.music {
            self.audio.music_cue(cue.as_deref());
            if let Some(m) = &mut self.mission {
                m.music_heard(cue);
            }
        }
        for unused in &news.unused {
            eprintln!("pause: {unused} kept in the menu; the alpha has nothing for it");
        }
        if let Some(c) = news.capture {
            self.capturing = c;
        }
        if let Some(table) = news.keys {
            table.apply(&mut config.keys);
            let path = config::Config::default_path();
            match std::fs::write(&path, config.to_toml()) {
                Ok(()) => eprintln!("pause: keys kept, saved to {}", path.display()),
                Err(e) => eprintln!("pause: could not save {}: {e}", path.display()),
            }
        }
        if let Some(s) = news.settings {
            self.keep_settings(&s, news.video, "pause", config);
        }
        if let Some(n) = news.save {
            let path = pause_save_path(n);
            eprintln!("pause: the game saved in slot {n}");
            self.save_to(&path);
        }
        if let Some(n) = news.load {
            eprintln!("pause: loading slot {n}");
            self.load_from(&pause_save_path(n));
            self.close_menu();
        }
        if news.quit {
            // The game goes back to its main menu: with the front end
            // (the whole game as it launches), the alpha starts again at
            // it; without, it closes.
            if self.flow.is_some() {
                eprintln!("pause: Quit: back to the front end");
                self.to_front_end = true;
            } else {
                eprintln!("pause: Quit: the alpha closes (it has no front end up)");
                event_loop.exit();
            }
        } else if news.resume && self.hud.menu {
            self.close_menu();
        }
    }

    /// The settings a menu (`menu`: the pause menu's or the front end's)
    /// kept: into the settings file, which is written (as the game saves
    /// its profile), and applied; `video` is its video screen's
    /// Resolution and Refresh Rate, if they were kept.
    fn keep_settings(
        &mut self,
        s: &pause::Settings,
        video: Option<pause::Video>,
        menu: &str,
        config: &mut Config,
    ) {
        config.keep_menu_settings(s);
        // The video screen's Resolution and Refresh Rate, as kept (only on
        // keeping: dropped changes leave the window be).
        if let Some(v) = video {
            Self::keep_video(&self.window, menu, v, config);
        }
        self.apply_options(config);
        config.save(menu);
    }

    /// Does what the front end's menus asked: their settings kept, their
    /// sounds, a save deleted, leaving, and Continue or Load Game: play
    /// starts from that save ([`Running::start_from`]).
    fn shell_news(&mut self, news: pause::News, config: &mut Config, event_loop: &ActiveEventLoop) {
        for cue in &news.stops {
            self.audio.stop_ui_sound(cue);
        }
        for cue in &news.sounds {
            self.audio.ui_sound(cue);
        }
        if let Some(v) = news.volumes {
            self.set_volumes(v);
        }
        for unused in &news.unused {
            eprintln!("front end: {unused} kept in the menu; the alpha has nothing for it");
        }
        if let Some(c) = news.capture {
            self.capturing = c;
        }
        if let Some(table) = news.keys {
            table.apply(&mut config.keys);
            let path = config::Config::default_path();
            match std::fs::write(&path, config.to_toml()) {
                Ok(()) => eprintln!("front end: keys kept, saved to {}", path.display()),
                Err(e) => eprintln!("front end: could not save {}: {e}", path.display()),
            }
        }
        if let Some(s) = news.settings {
            self.keep_settings(&s, news.video, "front end", config);
        } else if let Some(v) = news.video {
            Self::keep_video(&self.window, "front end", v, config);
            config.save("front end");
        }
        if let Some(n) = news.delete {
            let path = pause_save_path(n);
            match std::fs::remove_file(&path) {
                Ok(()) => eprintln!("front end: save {n} deleted ({})", path.display()),
                Err(e) => eprintln!("front end: could not delete {}: {e}", path.display()),
            }
        }
        if news.continue_game {
            match newest_save() {
                Some(n) => {
                    eprintln!("front end: Continue: slot {n}");
                    self.start_from(&pause_save_path(n), config);
                }
                None => eprintln!("front end: Continue: no saves"),
            }
        } else if let Some(n) = news.load {
            eprintln!("front end: Load Game: slot {n}");
            self.start_from(&pause_save_path(n), config);
        }
        if news.quit {
            eprintln!("front end: Exit");
            event_loop.exit();
        }
    }

    /// Starts play from the save at `path`, play not having started (the
    /// front end's Continue and Load Game): as its hero, with the world
    /// and the scripts' data from it as the scripts start
    /// ([`mission::MissionLink::set_save`]).
    fn start_from(&mut self, path: &std::path::Path, config: &mut Config) {
        let s = match save::SaveGame::read(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("front end: could not load: {e:#}");
                return;
            }
        };
        let hero = s.hero.unwrap_or(config.hero);
        config.hero = hero;
        if hero != self.game.scene.hero {
            eprintln!("hero: playing as {hero}");
            self.set_hero(hero);
        }
        let taken = self.mission.as_mut().is_some_and(|m| m.set_save(s));
        eprintln!(
            "front end: play starts from {}{}",
            path.display(),
            if taken {
                ""
            } else {
                " (world only: the scripts already run)"
            }
        );
        if !taken {
            self.load_from(path);
        }
    }

    /// The key or button a menu's keyboard table waited for: the front
    /// end's while it is up, else the pause menu's.
    fn captured(&mut self, control: input::Control) {
        match &mut self.flow {
            Some(flow) if flow.in_menus() => flow.captured(control),
            _ => self.hud.pause_captured.push(control),
        }
    }

    /// Plays as `hero` from now on: the body, clips, health and voice, and
    /// the scripts' player (the mission's and the hijacks'; the HUD's hears
    /// with its next state). What the scripts already chose by the old
    /// hero stays theirs.
    fn set_hero(&mut self, hero: Hero) {
        self.game.set_hero(&mut self.backend, hero);
        self.audio.hero = hero;
        let max = self.game.combat.max_hp;
        if let Some(m) = &mut self.mission {
            m.set_hero(hero, max);
        }
        if let Some(h) = &mut self.hijacks {
            h.set_hero(hero, max);
        }
    }

    fn save_game(&mut self) {
        self.save_to(&save::SaveGame::default_path());
    }

    /// Writes the game to `path`, with her cash, fuel and support stock as
    /// the scripts hold them.
    fn save_to(&mut self, path: &std::path::Path) {
        if write_save(&self.game, self.mission.as_ref(), path) {
            self.game.save_icon = game::SAVE_ICON_SECONDS;
        }
    }

    /// Loads the newest save: the player's own, or the scripts' autosave.
    fn load_game(&mut self) {
        self.load_from(&save::SaveGame::newest_path());
    }

    /// Loads the save at `path`.
    fn load_from(&mut self, path: &std::path::Path) {
        let path = path.to_path_buf();
        match save::SaveGame::read(&path) {
            // The scripts' part loads only into scripts starting afresh,
            // as the game loads a save into a world started again.
            Ok(s) if s.scripts.is_some() && self.mission.is_some() => {
                eprintln!(
                    "load: {} has the scripts' data: the game starts again from it",
                    path.display()
                );
                self.relaunch = Some(path);
            }
            Ok(mut s) => {
                let pmc = s.pmc.take();
                self.game.load(s);
                if let (Some(pmc), Some(m)) = (pmc, &mut self.mission) {
                    m.restore(&pmc);
                }
                eprintln!("game loaded from {}", path.display());
            }
            Err(e) => eprintln!("could not load: {e:#}"),
        }
    }

    /// Puts the cheat menu up ([`cheats`]), in the pause menu's place, its
    /// lists read from the game's data and the scripts' catalogue.
    fn show_cheats(&mut self) {
        let mut screen = cheats::CheatScreen::read(&self.game, self.mission.as_ref());
        screen.toggles = self.cheats;
        eprintln!("cheats: {}", screen.counts());
        self.hud.cheats = Some(screen);
        self.open_menu();
    }

    fn cheat_action(&mut self, action: cheats::CheatAction) {
        use cheats::CheatAction as A;
        match action {
            A::Close => self.close_menu(),
            A::Tab(tab) => {
                if let Some(c) = &mut self.hud.cheats {
                    c.tab = tab;
                    c.filter.clear();
                    c.said = None;
                }
            }
            A::Filter(f) => {
                if let Some(c) = &mut self.hud.cheats {
                    c.filter = f;
                }
            }
            action => {
                let done = cheats::apply(
                    &mut self.game,
                    self.mission.as_mut(),
                    &mut self.cheats,
                    action,
                );
                eprintln!("cheat: {}", done.said);
                if done.close {
                    self.close_menu();
                } else if let Some(c) = &mut self.hud.cheats {
                    c.toggles = self.cheats;
                    c.said = Some(done.said);
                }
            }
        }
    }

    /// Puts faction `faction`'s shop up ([`mission::SHOPS`]), in the pause
    /// menu's place: the world waits while she shops. Only once the
    /// scripts run.
    fn show_shop(&mut self, faction: usize, unlock_all: bool) {
        let Some((key, name)) = mission::SHOPS.get(faction) else {
            return;
        };
        let Some(view) = self
            .mission
            .as_mut()
            .and_then(|m| m.open_shop(key, unlock_all))
        else {
            eprintln!("shop: {name}'s shop isn't there until the scripts run");
            return;
        };
        eprintln!("shop: {name}'s, {} things on sale", view.items.len());
        self.hud.shop = Some(ui::ShopScreen {
            faction,
            view,
            said: None,
            unlock_all,
        });
        if !self.hud.menu {
            self.open_menu();
        }
    }

    fn shop_action(&mut self, action: ui::ShopAction, config: &mut Config) {
        let faction = self.hud.shop.as_ref().map_or(0, |s| s.faction);
        match action {
            ui::ShopAction::Close => self.close_menu(),
            ui::ShopAction::Faction(i) => {
                if let Some(m) = &mut self.mission {
                    m.close_shop();
                }
                self.show_shop(i, config.enhanced.shop.unlock_all);
            }
            ui::ShopAction::UnlockAll(on) => {
                config.enhanced.shop.unlock_all = on;
                self.show_shop(faction, on);
            }
            ui::ShopAction::Buy(id) => {
                let Some((view, bought)) = self.mission.as_mut().and_then(|m| m.buy(&id)) else {
                    return;
                };
                if let Some(shop) = &mut self.hud.shop {
                    let name = view
                        .items
                        .iter()
                        .find(|i| i.id == id)
                        .map_or(id.clone(), |i| i.name.clone());
                    shop.said = Some(if bought {
                        format!("Bought: {name}. Call it in from the support menu.")
                    } else {
                        format!("Could not buy {name}.")
                    });
                    shop.view = view;
                }
            }
        }
    }

    /// Writes the saves the scripts asked for: their autosaves after a
    /// contract (`Sys.RequestAutosave`) and checkpoints, each to its
    /// slot's file ([`save::SaveGame::slot_path`]). With `autosave` off
    /// (the pause menu's option) the autosaves are left out.
    fn answer_saves(&mut self, autosave: bool) {
        let Some(slots) = self.mission.as_mut().map(|m| m.take_saves()) else {
            return;
        };
        for slot in slots {
            if !autosave && slot == m2_game::api::AUTOSAVE {
                eprintln!("save: the scripts' autosave left out (Auto Save is off)");
                continue;
            }
            self.save_to(&save::SaveGame::slot_path(&slot));
        }
    }

    fn tick(&mut self, config: &mut Config, event_loop: &ActiveEventLoop) {
        // The front end's option screens and save list: the player's
        // settings and saves (asked once), and the window as it is.
        let settings = self
            .flow
            .as_ref()
            .is_some_and(flow::Flow::wants_settings)
            .then(|| self.pause_settings(config));
        if let Some(flow) = &mut self.flow {
            if let Some(s) = settings {
                flow.settings(s);
            }
            flow.window(display::current(&self.window));
            if let Ok(mut mixer) = self.audio.out.mixer.lock() {
                flow.tick(self.mission.as_mut(), &mut mixer);
            }
            let news = flow.take_news();
            // The hero picked in the front end, before the scripts start;
            // the pause menu offers them as the settings' hero.
            let picked = flow.take_pick();
            let holds = flow.holds();
            self.shell_news(news, config, event_loop);
            if let Some(hero) = picked {
                config.hero = hero;
                if hero != self.game.scene.hero {
                    eprintln!("hero: playing as {hero}");
                    self.set_hero(hero);
                }
            }
            if holds {
                // The world waits under the movie or the front end.
                self.input.clear();
                let (ear, facing) = self.game.listener();
                self.audio.update(ear, facing);
                return;
            }
        }
        if self.input.take_pressed(Action::Quit) {
            if self.hud.menu {
                self.close_menu();
            } else {
                self.hud.pause_settings = self.pause_settings(config);
                self.open_menu();
            }
        }
        if self.input.take_pressed(Action::Shop) && !self.hud.menu {
            let unlock = config.enhanced.shop.unlock_all;
            self.show_shop(0, unlock);
        }
        // The cheat menu: a developer's key (taken either way), or its
        // code typed in play by anyone.
        let typed = std::mem::take(&mut self.cheat_typed);
        if (self.input.take_pressed(Action::Cheats) && self.dev || typed) && !self.hud.menu {
            self.show_cheats();
        }
        if self.hud.menu != self.sound_held {
            // Paused, the world's sounds, the dialogue and the music playing
            // wait with the world; the menu's own sounds and music play.
            self.sound_held = self.hud.menu;
            self.audio.out.with(|m| {
                if self.sound_held {
                    use m2_engine::Category::{Chatter, Music, Vo, World};
                    m.hold(&[World, Chatter, Vo, Music], &[World, Chatter, Vo]);
                } else {
                    m.release();
                }
            });
        }
        if self.hud.menu {
            // Paused: the world waits.
            let (ear, facing) = self.game.listener();
            self.audio.update(ear, facing);
            return;
        }
        if self.input.take_pressed(Action::ToggleMouse) {
            self.released = self.mouse_look;
        }
        if self.input.take_pressed(Action::Fullscreen) {
            let how = self.display.toggle(&self.window, &mut config.window);
            eprintln!("display: F11: {how}");
            config.save("display");
        }
        // The developer's keys are taken either way, and do nothing
        // without `--dev` in the whole game's launch.
        let dev = self.dev;
        if !dev {
            self.input.take_pressed(Action::ToggleFly);
        }
        if self.input.take_pressed(Action::QuickSave) && dev {
            self.save_game();
        }
        if self.input.take_pressed(Action::QuickLoad) && dev {
            self.load_game();
        }
        // `--load-at`: the same load, by the clock, to test it in a window.
        if self.load_at.is_some_and(|at| Instant::now() >= at) {
            self.load_at = None;
            eprintln!("load: --load-at: loading as the pause menu does");
            self.load_from(&save::SaveGame::default_path());
        }
        // `--pause-at`: Escape, by the clock, to test pausing in a window.
        if self.pause_at.is_some_and(|at| Instant::now() >= at) && !self.hud.menu {
            self.pause_at = None;
            eprintln!("pause: --pause-at: opening the pause menu as Escape does");
            self.hud.pause_settings = self.pause_settings(config);
            self.open_menu();
        }
        if self.input.take_pressed(Action::ToggleMusic) {
            self.audio.toggle_music();
        }
        if self.input.take_pressed(Action::NextGait) && dev {
            self.game.scene.cycle_gaits();
        }
        if self.input.take_pressed(Action::ToggleCulling) && dev {
            self.culling = !self.culling;
            if let Err(e) = self
                .game
                .scene
                .materials
                .set_double_sided(&mut self.backend, !self.culling)
            {
                eprintln!("culling toggle failed: {e}");
            }
            eprintln!(
                "back-face culling {}",
                if self.culling { "on" } else { "off" }
            );
        }
        if let Some(h) = &mut self.hijacks {
            spans::time("hijacks", || {
                h.tick(&mut self.game, &mut self.input, TICK.as_secs_f32())
            });
        }
        // The pad, for a hijack the scripts play on a vehicle of theirs.
        if let Some(m) = &mut self.mission
            && m.hijack_prompts().is_some()
        {
            let (pressed, held) = hijack::pad_buttons(&mut self.input);
            m.hijack_pad(&pressed, &held);
        }
        // The grappling hook: on once the scripts allow it (or with
        // `--grapple`); the use key fires it at the helicopter in her
        // sights, or lets go of the line.
        let allowed = self.grapple || self.mission.as_ref().is_some_and(|m| m.grapple_enabled());
        grapple_keys(&mut self.game, &mut self.input, allowed);
        if let Some(m) = &mut self.mission {
            m.set_grappling(self.game.grapple.grappling());
        }
        spans::time("game", || {
            self.game.tick(
                &mut self.input,
                &config.camera,
                self.mouse_look,
                TICK.as_secs_f32(),
            );
        });
        cheats::keep_up(&mut self.game, self.cheats);
        if let Some(c) = &mut self.coop {
            c.tick(&mut self.game, &mut self.backend, TICK.as_secs_f32());
        }
        if let Some(h) = &mut self.hijacks {
            h.place(&mut self.game);
            h.seat(&mut self.game, TICK.as_secs_f32());
        }
        if let Some(m) = &mut self.mission {
            spans::time("scripts", || m.tick(&mut self.game, TICK.as_secs_f32()));
            self.game.dress(&mut self.backend);
            let (lines, hushed) = m.take_lines();
            for (id, cue, at) in lines {
                if let Some(seconds) = self.audio.say(id, &cue, at) {
                    m.line_length(id, seconds);
                }
            }
            for id in hushed {
                self.audio.hush(id);
            }
            for change in m.take_fades() {
                self.audio.change_category(change);
            }
            for (cue, stop) in m.take_sounds() {
                if stop {
                    self.audio.stop_ui_sound(&cue);
                } else {
                    self.audio.ui_sound(&cue);
                }
            }
            if let Some(cue) = m.take_music() {
                self.audio.music_cue_at(cue.as_deref(), m.music_at());
            }
            if let Some(preset) = m.take_reverb() {
                self.audio.set_reverb(preset);
            }
        }
        self.answer_saves(config.options.autosave);
        let (ear, facing) = self.game.listener();
        self.audio.update(ear, facing);
        // The ambience region the listener is in plays its own bed and
        // one-shots; until they are read, the town's or jungle's stand in.
        let zone = self.game.scene.world.as_ref().and_then(|w| {
            m2_engine::sound_regions::ambience_at(&w.index.sound_regions, ear.x, ear.z)
                .and_then(|r| r.ambience)
        });
        if !self.audio.zone_ambience(zone, ear, TICK.as_secs_f32())
            && self.audio.ambient_due(TICK.as_secs_f32())
        {
            let town = self.game.near_town();
            self.audio.play_ambient(town, ear);
        }
        for (i, shout, at) in self.game.scene.people.take_shouts() {
            let sets = match shout {
                npc::Shout::Alert => sound::VZ_SOLDIERS,
                npc::Shout::Flee if self.game.scene.people.is_woman(i) => sound::CIVILIAN_WOMEN,
                npc::Shout::Flee => sound::CIVILIANS,
            };
            self.audio.shout(sets[i % sets.len()], Some(at));
        }
        let sounds = self.game.combat.take_sounds();
        self.audio.play_fight(&sounds);
        let cues = self.game.scene.take_cues(ear, CUE_REACH);
        self.audio.play_cues(&cues);
        let engine = self.game.engine();
        self.audio.engine(engine, TICK.as_secs_f32());
        self.audio.rotors(&self.game.scene.flights.sounding());
        let driving = self.game.driving().is_some();
        let mut remarks = self.game.combat.take_remarks();
        if driving && !self.was_driving && self.game.car_remark_due() {
            remarks.push(combat::Remark::Vehicle);
        }
        if !driving {
            self.game.scripts_seated = false;
        }
        self.was_driving = driving;
        for remark in remarks {
            self.audio.remark(remark);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        // Back from the background: a new surface for the window, and play
        // goes on from where it waited.
        if let Some(state) = self.state.as_mut() {
            if std::mem::take(&mut state.background) {
                let size = state.window.inner_size();
                match state
                    .backend
                    .resume(state.window.clone(), size.width, size.height)
                {
                    Ok(()) => eprintln!("back from the background"),
                    Err(e) => {
                        eprintln!("back from the background: no surface ({e}); closing");
                        event_loop.exit();
                    }
                }
                state.last = Instant::now();
                state.audio.out.set_paused(false);
            }
            return;
        }
        match self.start(event_loop) {
            Ok(state) => self.state = Some(state),
            Err(e) => self.fail(event_loop, e),
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if let Some(path) = state.relaunch.take() {
            self.successor = relaunch(&path);
            event_loop.exit();
            return;
        }
        if std::mem::take(&mut state.to_front_end) {
            self.successor = relaunch_front_end();
            event_loop.exit();
            return;
        }
        // A game controller's buttons: Android reports them as keys of its
        // own, which go to the pad and nowhere else.
        if let WindowEvent::KeyboardInput { event: key, .. } = &event
            && let PhysicalKey::Unidentified(winit::keyboard::NativeKeyCode::Android(code)) =
                key.physical_key
            && state
                .pad
                .events
                .key(code, key.state == ElementState::Pressed)
        {
            return;
        }
        // Alt+Enter toggles fullscreen, as F11 does (the PC convention),
        // in play and menus alike; its Enter goes no further.
        // Alt from the modifiers and from the Alt keys themselves: a
        // ModifiersChanged can come before the Focused that would clear it.
        match &event {
            WindowEvent::ModifiersChanged(m) => state.alt = m.state().alt_key(),
            WindowEvent::KeyboardInput { event: key, .. }
                if matches!(
                    key.physical_key,
                    PhysicalKey::Code(KeyCode::AltLeft | KeyCode::AltRight)
                ) =>
            {
                state.alt = key.state == ElementState::Pressed;
            }
            _ => {}
        }
        if let WindowEvent::KeyboardInput { event: key, .. } = &event
            && state.alt
            && matches!(
                key.physical_key,
                PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter)
            )
        {
            if key.state == ElementState::Pressed && !key.repeat {
                let how = state.display.toggle(&state.window, &mut self.config.window);
                eprintln!("display: Alt+Enter: {how}");
                self.config.save("display");
            }
            return;
        }
        // Fingers on the screen in play are the touch controls'; in the
        // menus and the front end they are egui's (taps as clicks), and
        // whatever the controls held is let go.
        if let WindowEvent::Touch(t) = &event {
            let playing = !state.hud.menu && state.flow.as_ref().is_none_or(|f| !f.holds());
            if playing {
                let at = [t.location.x as f32, t.location.y as f32];
                match t.phase {
                    winit::event::TouchPhase::Started => state.pad.touch.down(t.id, at),
                    winit::event::TouchPhase::Moved => state.pad.touch.moved(t.id, at),
                    winit::event::TouchPhase::Ended | winit::event::TouchPhase::Cancelled => {
                        state.pad.touch.up(t.id);
                    }
                }
                return;
            }
            state.pad.touch.clear();
            // A tap skips a movie or presses start, as Enter does.
            if t.phase == winit::event::TouchPhase::Started
                && !state.hud.menu
                && let Some(flow) = state.flow.as_mut().filter(|f| f.holds())
            {
                flow.tap();
            }
        }
        // The menu takes the mouse and keys while it is open (Escape still
        // closes it).
        let to_menu = state.hud.menu
            && state.egui.on_window_event(&state.window, &event).consumed
            && !matches!(event, WindowEvent::KeyboardInput { .. });
        if to_menu {
            return;
        }
        if !state.hud.menu {
            // Keep egui's idea of the window (size, scale) current, and
            // hand it the keys: the HUD's movies that hold the controls
            // (the PDA, a fanfare's Enter) take them from its events.
            let _ = state.egui.on_window_event(&state.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.backend.resize(size.width, size.height);
                state.pad.touch.resize(size.width, size.height);
                state.display.resized(&state.window, size);
                let (sw, sh) = state.backend.size();
                eprintln!(
                    "display: resized: {}, swapchain {sw}x{sh}",
                    display::describe(&state.window)
                );
            }
            WindowEvent::Focused(focused) => {
                // Keys pressed or released elsewhere never reach us, so start
                // clean either way.
                state.focused = focused;
                if !focused {
                    state.alt = false;
                }
                state.input.clear();
                state.keep_mouse();
            }
            // The pause menu's keyboard table takes the next key or button.
            WindowEvent::KeyboardInput { event, .. } if state.focused && state.capturing => {
                if let PhysicalKey::Code(code) = event.physical_key
                    && event.state == ElementState::Pressed
                    && !event.repeat
                {
                    state.capturing = false;
                    state.captured(code.into());
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button,
                ..
            } if state.focused && state.capturing => {
                let control = match button {
                    winit::event::MouseButton::Left => Some(input::Mouse::Left),
                    winit::event::MouseButton::Right => Some(input::Mouse::Right),
                    winit::event::MouseButton::Middle => Some(input::Mouse::Middle),
                    _ => None,
                };
                if let Some(c) = control {
                    state.capturing = false;
                    state.captured(c.into());
                }
            }
            WindowEvent::MouseWheel { .. } if state.focused && state.capturing => {
                state.capturing = false;
                state.captured(input::Mouse::WheelUp.into());
            }
            WindowEvent::KeyboardInput { event, .. }
                if state.focused && state.flow.as_ref().is_some_and(flow::Flow::holds) =>
            {
                if let PhysicalKey::Code(code) = event.physical_key
                    && event.state == ElementState::Pressed
                    && !event.repeat
                    && let Some(flow) = &mut state.flow
                {
                    flow.key(code);
                }
            }
            // A dialog box the scripts put up (Fiona's briefing menu) takes
            // the menu keys, as the pad's buttons it reads; she stays put.
            WindowEvent::KeyboardInput { event, .. }
                if state.focused
                    && state
                        .mission
                        .as_ref()
                        .is_some_and(mission::MissionLink::dialog_open)
                    && matches!(event.physical_key, PhysicalKey::Code(c) if dialog_button(c).is_some()) =>
            {
                if let PhysicalKey::Code(code) = event.physical_key
                    && event.state == ElementState::Pressed
                    && !event.repeat
                    && let Some(b) = dialog_button(code)
                {
                    state.dialog_keys.push(b);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if state.focused => {
                if let PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) = event.physical_key
                    && event.state == ElementState::Pressed
                    && !event.repeat
                {
                    state.buttons.push("selection".into());
                }
                if let PhysicalKey::Code(code) = event.physical_key {
                    if event.state == ElementState::Pressed
                        && !event.repeat
                        && state.cheat_code.key(input::letter(code))
                    {
                        eprintln!("cheats: code typed");
                        state.cheat_typed = true;
                    }
                    state.input.key(
                        &self.config.keys,
                        code,
                        event.state == ElementState::Pressed,
                        event.repeat,
                    );
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: winit::event::MouseButton::Right | winit::event::MouseButton::Left,
                ..
            } if !state.mouse_look && !state.hud.menu => {
                state.released = false;
                state.input.mouse_used();
            }
            // With the mouse captured, its buttons and wheel do what they
            // are bound to (by default the PC game's: fire, the secondary
            // attack, melee, and the wheel switches weapon).
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } if state.mouse_look && state.focused => {
                let control = match button {
                    winit::event::MouseButton::Left => Some(input::Mouse::Left),
                    winit::event::MouseButton::Right => Some(input::Mouse::Right),
                    winit::event::MouseButton::Middle => Some(input::Mouse::Middle),
                    _ => None,
                };
                if let Some(c) = control {
                    let down = pressed == ElementState::Pressed;
                    if down {
                        state.input.mouse_used();
                    }
                    state.input.key(&self.config.keys, c, down, false);
                }
            }
            WindowEvent::MouseWheel { delta, .. } if state.mouse_look && state.focused => {
                let up = match delta {
                    winit::event::MouseScrollDelta::LineDelta(_, y) => y,
                    winit::event::MouseScrollDelta::PixelDelta(p) => p.y as f32,
                };
                if up != 0.0 {
                    let c = if up > 0.0 {
                        input::Mouse::WheelUp
                    } else {
                        input::Mouse::WheelDown
                    };
                    state.input.mouse_used();
                    state.input.key(&self.config.keys, c, true, false);
                    state.input.key(&self.config.keys, c, false, false);
                }
            }
            WindowEvent::RedrawRequested => {
                // The swapchain at the window's size now: going in and out
                // of fullscreen, the `Resized` events come late and carry
                // sizes on the way, so a frame could be drawn at a size the
                // window has left, in a corner of it.
                let size = state.window.inner_size();
                if (size.width, size.height) != state.backend.size() {
                    state.backend.resize(size.width, size.height);
                }
                if let Err(e) = state.frame(&mut self.config, event_loop) {
                    self.fail(event_loop, e);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        match event {
            DeviceEvent::MouseMotion { delta } => state.input.mouse_motion(delta.0, delta.1),
            // A game controller's sticks, triggers and hat (Android's AXIS_*
            // numbers, from the patched winit of the Android build).
            DeviceEvent::Motion { axis, value } if cfg!(target_os = "android") => {
                state.pad.events.axis(axis, value as f32);
            }
            _ => {}
        }
    }

    /// Android takes the window's surface away when the app goes to the
    /// background: the renderer lets it go and keeps all else, and play
    /// waits (no ticks, no sound) until [`resumed`](Self::resumed).
    fn suspended(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = self.state.as_mut() {
            eprintln!("sent to the background: play waits");
            state.backend.suspend();
            state.background = true;
            state.audio.out.set_paused(true);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_ref() else {
            return;
        };
        // In the background no frames are asked for, and the loop sleeps
        // until something happens: play waits.
        if state.background {
            event_loop.set_control_flow(ControlFlow::Wait);
            return;
        }
        // With a frame rate cap (the phone's, for its battery), the loop
        // sleeps out the rest of each frame's time.
        let cap = self.config.window.max_fps;
        if cap > 0 {
            let due = state.last + Duration::from_secs_f64(1.0 / f64::from(cap));
            if Instant::now() < due {
                event_loop.set_control_flow(ControlFlow::WaitUntil(due));
                return;
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
        state.window.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starting_again_from_a_save_keeps_the_launch_but_its_load() {
        let args = [
            "--alpha",
            "--load",
            "old.toml",
            "--mission",
            "--load",
            "older.toml",
            "--load-at",
            "30",
        ]
        .map(String::from);
        assert_eq!(
            args_loading(args, std::path::Path::new("new.toml")),
            ["--alpha", "--mission", "--load", "new.toml"]
        );
    }

    #[test]
    fn four_by_three_keeps_its_shape_on_any_window() {
        let mut w = config::WindowConfig::default();
        assert_eq!(view_aspect(&w, 1920, 1080), 1920.0 / 1080.0);
        w.wide = false;
        assert_eq!(view_aspect(&w, 1920, 1080), 4.0 / 3.0);
    }

    #[test]
    fn quitting_to_the_front_end_keeps_the_launch_but_its_load() {
        let args = ["--load", "old.toml", "--size", "1280x720", "--to-front-end"].map(String::from);
        assert_eq!(
            args_front_end(args),
            ["--size", "1280x720", "--to-front-end"]
        );
        assert_eq!(args_front_end(Vec::new()), ["--to-front-end"]);
    }

    #[test]
    fn pace_logs_each_span_with_its_slowest_frame() {
        let mut pace = Pace::default();
        let t0 = Instant::now();
        assert!(pace.frame(t0, (1280, 720)).is_none());
        let mut t = t0;
        let mut line = None;
        // 16 ms frames, one of 80 ms, until the span is over.
        for k in 0.. {
            t += Duration::from_millis(if k == 10 { 80 } else { 16 });
            line = pace.frame(t, (1280, 720)).map(|(l, _)| l);
            if line.is_some() {
                break;
            }
        }
        let line = line.unwrap();
        assert!(line.starts_with("pace: 6"), "{line}");
        assert!(
            line.contains("slowest frame 80 ms, 1 over 50 ms, 1280x720"),
            "{line}"
        );
        // The next span starts clean.
        t += Duration::from_millis(16);
        assert!(pace.frame(t, (1280, 720)).is_none());
        assert_eq!((pace.frames, pace.slow), (1, 0));
    }
}
