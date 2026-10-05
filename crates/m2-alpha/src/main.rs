//! `m2-alpha`: the native game, playing from the player's own install.
//!
//! ```text
//! m2-alpha [--install <dir>] [--spawn <x>,<z>] [--model <hash>]... [--gait <gait>]
//!          [--config <file>]
//!          [--size <w>x<h>] [--vsync on|off] [--fullscreen] [--double-sided] [--fly]
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
//! (`--spawn` picks where to start; the default is the first town). WASD
//! runs, Shift sprints, Q held walks, Space jumps. E gets in and out of
//! the vehicles parked ahead of the spawn (WASD drive, Space brakes, R
//! rights an overturned one). V switches to a
//! free-flying camera and back (`--fly` starts flying): WASD/Space/C fly,
//! Shift speeds up. With `--model` it shows those models on a ground grid
//! instead, characters playing the game's locomotion clips in place (N
//! steps through idle, walk, run and sprint; `--gait` picks the first).
//! Tab or the right mouse button captures the mouse, F11 toggles
//! fullscreen, F3 toggles back-face culling, Escape releases the mouse,
//! then quits.
//!
//! `--screenshot` on the island waits for the ground, lands the player and,
//! with `--gait walk|run|sprint`, runs her forward for three seconds before
//! taking the picture, printing where she got to. With `--drive` she gets
//! in the nearest parked vehicle instead and drives it forward; with
//! `--fight` she shoots the nearest soldier, fires a rocket at a vehicle and
//! throws a grenade.

mod autoplay;
mod cache;
mod camera;
mod combat;
mod config;
mod flights;
mod game;
mod input;
mod mission;
mod npc;
mod player;
mod props;
mod route;
mod save;
mod scene;
mod sound;
mod startup;
mod ui;

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use m2_engine::{Gait, Install, WorldSettings};
use m2_render::gpu::WgpuBackend;
use m2_render::{DrawQueue, RenderBackend};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, DeviceId, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::PhysicalKey;
use winit::window::{CursorGrabMode, Fullscreen, Window, WindowId};

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
    gait: Option<Gait>,
    double_sided: bool,
    fly: bool,
    drive: bool,
    /// With `--screenshot` on foot: jump just before the picture.
    jump: bool,
    /// With `--screenshot` on foot: throw a grenade just before it.
    throw: bool,
    /// With `--jump` or `--throw`: seconds before the picture she starts.
    lead: Option<f32>,
    fight: bool,
    /// With `--screenshot`: rocket the nearest fuel tank or barrel.
    blast: bool,
    /// With `--drive`: wreck it at speed at the end and watch it stop.
    wreck: bool,
    /// With `--drive`: a vehicle whose name holds this.
    vehicle: Option<String>,
    /// With `--drive`: swing the camera this far (radians) before the picture.
    look: f32,
    /// With `--screenshot`: how many of the nearest spawns to list.
    list_spawns: usize,
    /// With `--fight`: seconds to wait after it before the picture.
    fight_wait: f32,
    /// With `--screenshot` on foot: face this way (radians from +z towards
    /// +x) before the picture.
    face: Option<f32>,
    /// With `--screenshot` on foot: hold the weapon up to aim at the end.
    aim: bool,
    /// With `--screenshot` on foot: the weapon she has up.
    weapon: Option<combat::Weapon>,
    /// Weapons held as worked out from the hands (`--grip hands`), not
    /// on the hand's attach node as posed.
    hands_grip: bool,
    /// With `--screenshot` on foot: take the picture close up, from her
    /// right and a little ahead, at chest height, to see her hands.
    closeup: bool,
    /// With `--screenshot`: cast rays around where she stands and list
    /// what they hit, to find unseen obstacles.
    probe: bool,
    /// With `--aim` (or `--drive --look`): fire just before the picture.
    fire: bool,
    /// With `--mission --screenshot`: take the picture from this far from
    /// the nearest marker, looking at it.
    mission_look: Option<f32>,
    mission: bool,
    mission_drivers: Vec<(String, String)>,
    mission_seconds: f32,
    /// With `--mission --screenshot`: she plays the mission herself, by
    /// its markers, until it is done or the seconds are up.
    autoplay: bool,
    sea: Option<f32>,
    size: Option<(u32, u32)>,
    vsync: Option<bool>,
    fullscreen: bool,
    screenshot: Option<PathBuf>,
    write_config: bool,
}

fn parse_args() -> anyhow::Result<Options> {
    let mut o = Options::default();
    let mut args = std::env::args().skip(1);
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
            "--double-sided" => o.double_sided = true,
            "--fly" => o.fly = true,
            "--drive" => o.drive = true,
            "--jump" => o.jump = true,
            "--throw" => o.throw = true,
            "--lead" => o.lead = Some(value()?.parse().context("--lead expects seconds")?),
            "--fight" => o.fight = true,
            "--blast" => o.blast = true,
            "--wreck" => o.wreck = true,
            "--vehicle" => o.vehicle = Some(value()?.to_ascii_lowercase()),
            "--look" => {
                o.look = value()?
                    .parse::<f32>()
                    .context("--look expects degrees")?
                    .to_radians()
            }
            "--mission" => o.mission = true,
            "--mission-driver" => {
                let path = value()?;
                let code =
                    std::fs::read_to_string(&path).with_context(|| format!("reading {path}"))?;
                o.mission_drivers.push((path, code));
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
            "--aim" => o.aim = true,
            "--weapon" => {
                o.weapon = Some(match value()?.as_str() {
                    "rifle" => combat::Weapon::Rifle,
                    "rpg" => combat::Weapon::Rpg,
                    v => bail!("--weapon {v}: expected rifle or rpg"),
                })
            }
            "--closeup" => o.closeup = true,
            "--grip" => {
                o.hands_grip = match value()?.as_str() {
                    "attach" => false,
                    "hands" => true,
                    v => bail!("--grip {v}: expected attach or hands"),
                }
            }
            "--probe" => o.probe = true,
            "--fire" => o.fire = true,
            "--mission-look" => {
                o.mission_look = Some(value()?.parse().context("--mission-look expects metres")?)
            }
            "--fight-wait" => {
                o.fight_wait = value()?.parse().context("--fight-wait expects seconds")?
            }
            "--autoplay" => o.autoplay = true,
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

fn main() -> anyhow::Result<()> {
    keep_crash_reports();
    let options = parse_args()?;
    combat::use_attach_frame(!options.hands_grip);
    let config_path = options
        .config_path
        .clone()
        .unwrap_or_else(Config::default_path);
    let mut config = Config::load(&config_path)?;
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
    config.window.fullscreen |= options.fullscreen;
    if let Some(sea) = options.sea {
        config.world.sea_level = sea;
    }
    if options.write_config {
        print!("{}", config.to_toml());
        return Ok(());
    }
    let install = match &config.install {
        Some(dir) => Some(Install::find(dir)?),
        None => match Install::detect() {
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
        Mode::Island(options.spawn.unwrap_or(scene::DEFAULT_SPAWN))
    } else {
        Mode::Models(Showcase {
            models: vec![scene::DEFAULT_MODEL],
            gait,
            clips: Vec::new(),
            turn: 0.0,
        })
    };

    if let Some(out) = &options.screenshot {
        return screenshot(&config, install.as_ref(), &mode, &options, out);
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        config,
        install,
        mode,
        double_sided: options.double_sided,
        fly: options.fly,
        mission: options.mission,
        mission_drivers: options.mission_drivers.clone(),
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// What to show.
#[derive(Clone, Debug)]
enum Mode {
    /// The streamed island, starting above (x, z).
    Island((f32, f32)),
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
) -> anyhow::Result<Game> {
    let mut scene = Scene::new(backend)?;
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
                scene.open_world(backend, install, settings, *spawn)?;
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
            game.startup = Some(startup::Startup::start(install));
        }
    }
    Ok(game)
}

/// What the HUD shows: the game's, with the mission's objectives and the
/// view their markers are placed with.
fn hud_info(
    game: &Game,
    mission: Option<&mission::MissionLink>,
    fov: f32,
    aspect: f32,
) -> ui::HudInfo {
    let mut info = game.hud_info();
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
    let mut view = game.view(fov, aspect);
    if game.scene.world.is_some() {
        view.lighting.fog_density = 0.00025;
    }
    let focus = game.focus();
    game.scene.update(backend, focus)?;
    let mut queue = DrawQueue::new();
    backend.begin_frame(&view)?;
    game.scene.draw(backend, &mut queue, &view, &game.combat)?;
    queue.flush(&view, backend)?;
    match hud {
        Some(hud) => backend.end_frame_with(&mut |t| hud.paint(t))?,
        None => backend.end_frame()?,
    }
    Ok(())
}

/// Renders one frame offscreen and writes it as a binary PPM. On the
/// island, once it has streamed in, the player lands and (with `gait`
/// walk, run or sprint) heads forward for three seconds first.
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
    )?;
    if options.fly {
        game.on_foot = false;
    }
    let fov = config.camera.fov;
    let aspect = w as f32 / h as f32;
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
        "people: {} kinds, {} about ({} soldiers, {} patrolling, {} landed) after {:.1} s",
        people.kinds.len(),
        people.npcs.len(),
        people.npcs.iter().filter(|n| n.spawn.is_some()).count(),
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
            let kind = s
                .template
                .as_deref()
                .and_then(|t| people.kind_for(t))
                .or_else(|| people.kind_for(&s.name));
            eprintln!(
                "  spawn {:08x} {:?} {d:.0} m at {:.0}, template {:?}, layer {} ({}), {} route points, {}{}",
                s.entity,
                s.name,
                s.position,
                s.template,
                s.layer.map_or("static".to_owned(), |l| format!("{l:08x}")),
                if people.layer_in(s.layer) {
                    "in"
                } else {
                    "out"
                },
                s.route.len(),
                if kind.is_some() {
                    "a soldier"
                } else if people.mission_soldiers.contains(&s.entity) {
                    "a mission soldier"
                } else {
                    "nobody"
                },
                if people.npcs.iter().any(|n| n.spawn == Some(s.entity)) {
                    ", standing"
                } else {
                    ""
                }
            );
        }
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
    }
    if options.fight && game.on_foot && game.scene.player.is_some() {
        fight_for_screenshot(&mut backend, &mut game, fov, aspect, options.fight_wait)?;
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
        // A jump or throw starts this many ticks before the picture.
        let lead = (options.lead.unwrap_or(0.2) * 60.0)
            .round()
            .clamp(1.0, 179.0) as usize;
        for i in 0..180 {
            if let (Some(face), Some(p)) = (options.face, game.scene.player.as_mut()) {
                p.yaw = face;
                if i == 0 {
                    p.facing = face;
                }
            }
            // A jump takes the picture on the way up (by default), a
            // throw with the grenade still in her hand; aiming, the
            // weapon comes up for the last second.
            let jump = options.jump && i == 180 - lead;
            let trigger = combat::Trigger {
                aim: options.aim && i >= 120,
                // The last shot's flash is still up for the picture.
                fire: options.fire && i == 177,
                throw: options.throw && i == 180 - lead,
                select: options.weapon.filter(|_| i == 0),
            };
            game.step(
                Intent {
                    jump,
                    hold_jump: jump,
                    trigger,
                    ..intent
                },
                tick,
            );
            if i % 30 == 0 {
                // Keep streaming around her as she goes.
                render(&mut backend, &mut game, fov, aspect, None)?;
            }
        }
        if let (Some(p), Some(from)) = (&game.scene.player, from) {
            let hash = |c: usize| p.model.as_ref().map_or(0, |(_, r)| r.clips[c].hash);
            eprintln!(
                "player: {:?} from {from:.2} to {:.2}, grounded {}, clip {:?}, upper {:?}, throwing {:?}, grenade in hand {}, grenades out {}",
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
                    .filter(|(rocket, ..)| !rocket)
                    .count(),
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
    if let (Some(reach), Some(link)) = (options.mission_look, &link) {
        look_at_marker(&mut backend, &mut game, link, reach, fov, aspect)?;
    }
    // The picture, with the HUD.
    let mut hud = ui::Hud::new();
    let mut settings = config.clone();
    let info = hud_info(&game, link.as_ref(), fov, aspect);
    let _ = hud.build(ui::headless_input(w, h, 1.0), &info, &mut settings);
    render(&mut backend, &mut game, fov, aspect, Some(&mut hud))?;
    write_ppm(&backend, out)?;
    eprintln!("wrote {}", out.display());
    Ok(())
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
    let Some((at, label, _)) = link
        .view()
        .markers
        .into_iter()
        .min_by(|a, b| a.0.distance(here).total_cmp(&b.0.distance(here)))
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
    let mut link = mission::MissionLink::start(install, options.mission_drivers.clone());
    let started = Instant::now();
    while !link.running() && started.elapsed() < Duration::from_secs(300) {
        link.tick(game, tick);
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
    // And the start's boat from the chase camera five seconds after she
    // is seated in it (`-boat`), and her three seconds after she gets out
    // of it (`-ashore`): the tick each is due.
    let mut boat_shot: Option<u32> = None;
    let mut ashore_shot: Option<u32> = None;
    for i in 0..(seconds / tick).round() as u32 {
        let intent = match &mut auto {
            Some(a) => {
                let marks = link.marks();
                let objectives = link.view().objectives;
                a.step(game, &marks, &objectives, tick)
            }
            None => Intent::default(),
        };
        game.step(intent, tick);
        link.tick(game, tick);
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
                boat_shot.get_or_insert(i + (5.0 / tick) as u32);
            } else if boat_shot.is_some() {
                ashore_shot.get_or_insert(i + (3.0 / tick) as u32);
            }
            for (due, name) in [(boat_shot, "boat"), (ashore_shot, "ashore")] {
                if due == Some(i) {
                    render(backend, game, fov, aspect, None)?;
                    write_ppm(backend, &picture(name.into()))?;
                    eprintln!("autoplay: pictured {name} at {:.1}", game.focus());
                }
            }
        }
        if auto.is_some() && link.completed() {
            eprintln!(
                "autoplay: contract completed after {:.1} s of play ({:.0} s here)",
                i as f32 * tick,
                played.elapsed().as_secs_f32()
            );
            break;
        }
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
        .filter(|n| people.kinds.get(n.kind).is_some_and(|k| k.named))
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
            (Some(w), Some((id, hull))) => autoplay::steer_round(&w.physics, id, hull),
            _ => 0.0,
        };
        let throttle = Intent {
            movement: m2_math::glam::Vec2::new(steer, 1.0),
            ..Intent::default()
        };
        game.step(throttle, tick);
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
            let look = if i < swing {
                m2_math::glam::Vec2::new(options.look / swing as f32, 0.0)
            } else {
                m2_math::glam::Vec2::ZERO
            };
            let trigger = combat::Trigger {
                fire: options.fire && i == 89,
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
                .filter(|(rocket, ..)| *rocket)
                .count()
        ),
        (Some(m), _, None) => eprintln!("drive: fire: muzzle {m:.2}, but no flash"),
        (Some(m), None, _) => eprintln!("drive: fire: muzzle {m:.2}, vehicle not placed"),
    }
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
    for _ in 0..120 {
        game.step(Intent::default(), tick);
    }
    let spent = game.scene.explosives.iter().filter(|e| e.gone).count();
    eprintln!(
        "blast: {} blasts, the last at {:.1?}; {spent} of {} explosives spent; player hp {:.0}",
        game.combat.blasts,
        game.combat.last_blast,
        game.scene.explosives.len(),
        game.combat.hp
    );
    for i in 0..(wait / tick) as usize {
        game.step(Intent::default(), tick);
        if i % 30 == 0 {
            render(backend, game, fov, aspect, None)?;
        }
    }
    aim_at(game, at);
    game.step(Intent::default(), tick);
    render(backend, game, fov, aspect, None)?;
    Ok(())
}

struct Running {
    window: Arc<Window>,
    backend: WgpuBackend,
    game: Game,
    audio: sound::GameAudio,
    /// Whether she was driving last tick (getting in is worth a word).
    was_driving: bool,
    input: Input,
    mouse_look: bool,
    /// Keyboard input is ignored while another window has focus.
    focused: bool,
    culling: bool,
    last: Instant,
    lag: Duration,
    fps: (Instant, u32),
    title: String,
    hud: ui::Hud,
    egui: egui_winit::State,
    mission: Option<mission::MissionLink>,
}

struct App {
    config: Config,
    install: Option<Install>,
    mode: Mode,
    /// Start with back-face culling off.
    double_sided: bool,
    /// Start flying rather than on foot.
    fly: bool,
    /// Run the mission scripts.
    mission: bool,
    /// Lua to run before they start (`--mission-driver`).
    mission_drivers: Vec<(String, String)>,
    state: Option<Running>,
    error: Option<anyhow::Error>,
}

impl App {
    fn start(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<Running> {
        let w = &self.config.window;
        let mut attributes = Window::default_attributes()
            .with_title("Mercenaries 2 (alpha)")
            .with_inner_size(PhysicalSize::new(w.width, w.height));
        if w.fullscreen {
            attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(event_loop.create_window(attributes)?);
        let size = window.inner_size();
        let mut backend =
            WgpuBackend::for_window(window.clone(), size.width, size.height, w.vsync)?;
        let adapter = backend.adapter_summary();
        eprintln!("adapter: {adapter:?}");
        let mut game = build_scene(
            &mut backend,
            self.install.as_ref(),
            &self.mode,
            self.double_sided,
            self.config.world.sea_level,
            self.mission,
        )?;
        if self.fly {
            game.on_foot = false;
        }
        let focused = window.has_focus();
        let hud = ui::Hud::new();
        let egui = egui_winit::State::new(
            hud.ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            None,
        );
        Ok(Running {
            title: format!(
                "Mercenaries 2 (alpha) - {} on {}",
                adapter.name, adapter.backend
            ),
            window,
            backend,
            game,
            audio: sound::GameAudio::start(self.install.as_ref()),
            was_driving: false,
            input: Input::default(),
            mouse_look: false,
            focused,
            culling: !self.double_sided,
            last: Instant::now(),
            lag: Duration::ZERO,
            fps: (Instant::now(), 0),
            egui,
            hud,
            mission: match (&self.install, &self.mode) {
                (Some(install), Mode::Island(_)) if self.mission => Some(
                    mission::MissionLink::start(install, self.mission_drivers.clone()),
                ),
                _ => None,
            },
        })
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, e: anyhow::Error) {
        eprintln!("error: {e:#}");
        self.error = Some(e);
        event_loop.exit();
    }
}

impl Running {
    fn set_mouse_look(&mut self, on: bool) {
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

    /// Advances the simulation by whole ticks (none while the menu is
    /// open) and draws a frame with the HUD.
    fn frame(&mut self, config: &mut Config, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let now = Instant::now();
        self.lag += (now - self.last).min(MAX_FRAME);
        self.last = now;
        while self.lag >= TICK {
            self.lag -= TICK;
            self.tick(config, event_loop);
        }
        let vsync = config.window.vsync;
        let input = self.egui.take_egui_input(&self.window);
        let (w, h) = self.backend.size();
        let info = hud_info(
            &self.game,
            self.mission.as_ref(),
            config.camera.fov,
            w as f32 / h as f32,
        );
        let (action, output) = self.hud.build(input, &info, config);
        self.egui.handle_platform_output(&self.window, output);
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
            Some(ui::MenuAction::Quit) => event_loop.exit(),
            None => {}
        }
        let (w, h) = self.backend.size();
        render(
            &mut self.backend,
            &mut self.game,
            config.camera.fov,
            w as f32 / h as f32,
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
        Ok(())
    }

    fn open_menu(&mut self) {
        self.hud.menu = true;
        self.set_mouse_look(false);
        self.input.clear();
    }

    fn close_menu(&mut self) {
        self.hud.menu = false;
        self.input.clear();
        self.set_mouse_look(true);
    }

    fn save_game(&mut self) {
        let path = save::SaveGame::default_path();
        match self.game.save() {
            Some(s) => match s.write(&path) {
                Ok(()) => eprintln!("game saved to {}", path.display()),
                Err(e) => eprintln!("could not save: {e:#}"),
            },
            None => eprintln!("nothing to save until the island has loaded"),
        }
    }

    fn load_game(&mut self) {
        match save::SaveGame::read(&save::SaveGame::default_path()) {
            Ok(s) => {
                self.game.load(s);
                eprintln!("game loaded");
            }
            Err(e) => eprintln!("could not load: {e:#}"),
        }
    }

    fn tick(&mut self, config: &Config, _event_loop: &ActiveEventLoop) {
        if self.input.take_pressed(Action::Quit) {
            if self.hud.menu {
                self.close_menu();
            } else {
                self.open_menu();
            }
        }
        if self.hud.menu {
            // Paused: the world waits, sound carries on.
            let (ear, facing) = self.game.listener();
            self.audio.update(ear, facing);
            return;
        }
        if self.input.take_pressed(Action::ToggleMouse) {
            self.set_mouse_look(!self.mouse_look);
        }
        if self.input.take_pressed(Action::Fullscreen) {
            let full = self.window.fullscreen().is_none();
            self.window
                .set_fullscreen(full.then_some(Fullscreen::Borderless(None)));
        }
        if self.input.take_pressed(Action::QuickSave) {
            self.save_game();
        }
        if self.input.take_pressed(Action::QuickLoad) {
            self.load_game();
        }
        if self.input.take_pressed(Action::ToggleMusic) {
            self.audio.toggle_music();
        }
        if self.input.take_pressed(Action::NextGait) {
            self.game.scene.cycle_gaits();
        }
        if self.input.take_pressed(Action::ToggleCulling) {
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
        self.game.tick(
            &mut self.input,
            &config.camera,
            self.mouse_look,
            TICK.as_secs_f32(),
        );
        if let Some(m) = &mut self.mission {
            m.tick(&mut self.game, TICK.as_secs_f32());
        }
        let (ear, facing) = self.game.listener();
        self.audio.update(ear, facing);
        if self.audio.ambient_due(TICK.as_secs_f32()) {
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
        let engine = self.game.engine();
        self.audio.engine(engine, TICK.as_secs_f32());
        self.audio.rotors(&self.game.scene.flights.sounding());
        let driving = self.game.driving().is_some();
        let mut remarks = self.game.combat.take_remarks();
        if driving && !self.was_driving {
            remarks.push(combat::Remark::Vehicle);
        }
        self.was_driving = driving;
        for remark in remarks {
            self.audio.remark(remark);
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_some() {
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
        // The menu takes the mouse and keys while it is open (Escape still
        // closes it).
        let to_menu = state.hud.menu
            && state.egui.on_window_event(&state.window, &event).consumed
            && !matches!(event, WindowEvent::KeyboardInput { .. });
        if to_menu {
            return;
        }
        if !state.hud.menu && !matches!(event, WindowEvent::KeyboardInput { .. }) {
            // Keep egui's idea of the window (size, scale) current.
            let _ = state.egui.on_window_event(&state.window, &event);
        }
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => state.backend.resize(size.width, size.height),
            WindowEvent::Focused(focused) => {
                // Keys pressed or released elsewhere never reach us, so start
                // clean either way.
                state.focused = focused;
                state.input.clear();
                if !focused {
                    state.set_mouse_look(false);
                }
            }
            WindowEvent::KeyboardInput { event, .. } if state.focused => {
                if let PhysicalKey::Code(code) = event.physical_key {
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
            } if !state.mouse_look && !state.hud.menu => state.set_mouse_look(true),
            // With the mouse captured, the buttons fire and aim.
            WindowEvent::MouseInput {
                state: pressed,
                button,
                ..
            } if state.mouse_look && state.focused => {
                let action = match button {
                    winit::event::MouseButton::Left => Some(Action::Fire),
                    winit::event::MouseButton::Right => Some(Action::Aim),
                    _ => None,
                };
                if let Some(a) = action {
                    state.input.button(a, pressed == ElementState::Pressed);
                }
            }
            WindowEvent::RedrawRequested => {
                if let Err(e) = state.frame(&mut self.config, event_loop) {
                    self.fail(event_loop, e);
                }
            }
            _ => {}
        }
    }

    fn device_event(&mut self, _event_loop: &ActiveEventLoop, _id: DeviceId, event: DeviceEvent) {
        if let (Some(state), DeviceEvent::MouseMotion { delta }) = (self.state.as_mut(), event) {
            state.input.mouse_motion(delta.0, delta.1);
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &self.state {
            state.window.request_redraw();
        }
    }
}
