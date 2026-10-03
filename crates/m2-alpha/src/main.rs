//! `m2-alpha`: the native game, playing from the player's own install.
//!
//! ```text
//! m2-alpha [--install <dir>] [--spawn <x>,<z>] [--model <hash>]... [--gait <gait>]
//!          [--config <file>]
//!          [--size <w>x<h>] [--vsync on|off] [--fullscreen] [--double-sided] [--fly]
//!          [--screenshot <out.ppm> [--drive]] [--write-config]
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
//! in the nearest parked vehicle instead and drives it forward.

mod camera;
mod config;
mod game;
mod input;
mod player;
mod scene;
mod sound;

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
    spawn: Option<(f32, f32)>,
    gait: Option<Gait>,
    double_sided: bool,
    fly: bool,
    drive: bool,
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

fn main() -> anyhow::Result<()> {
    let options = parse_args()?;
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
    if options.write_config {
        print!("{}", config.to_toml());
        return Ok(());
    }
    let install = match &config.install {
        Some(dir) => Some(Install::find(dir)?),
        None => {
            eprintln!(
                "No install configured: set `install` in {} or pass --install <dir>. Showing the ground only.",
                config_path.display()
            );
            None
        }
    };
    // No models named: the island (or, without an install, Jennifer's
    // grid, which shows only the ground).
    let gait = options.gait.unwrap_or(Gait::Idle);
    let mode = if !options.models.is_empty() {
        Mode::Models(options.models.clone(), gait)
    } else if install.is_some() {
        Mode::Island(options.spawn.unwrap_or(scene::DEFAULT_SPAWN))
    } else {
        Mode::Models(vec![scene::DEFAULT_MODEL], gait)
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
    Models(Vec<u32>, Gait),
}

/// Builds the scene on `backend` and points a camera at it.
fn build_scene(
    backend: &mut dyn RenderBackend,
    install: Option<&Install>,
    mode: &Mode,
    double_sided: bool,
) -> anyhow::Result<Game> {
    let mut scene = Scene::new(backend)?;
    if double_sided {
        scene.materials.set_double_sided(backend, true)?;
    }
    if let Some(install) = install {
        match mode {
            Mode::Island(spawn) => {
                scene.open_world(backend, install, WorldSettings::default(), *spawn)?;
            }
            Mode::Models(models, gait) => {
                scene.open(install)?;
                if let Err(e) = scene.load_models(backend, models, *gait) {
                    eprintln!("could not load models: {e:#}");
                }
            }
        }
    }
    let (eye, target) = scene.focus;
    Ok(Game::new(scene, FlyCamera::looking_at(eye, target)))
}

/// Streams around the player (or camera) and draws a frame.
fn render(
    backend: &mut dyn RenderBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
) -> anyhow::Result<()> {
    let mut view = game.view(fov, aspect);
    if game.scene.world.is_some() {
        view.lighting.fog_density = 0.00025;
    }
    let focus = game.focus();
    game.scene.update(backend, focus)?;
    let mut queue = DrawQueue::new();
    backend.begin_frame(&view)?;
    game.scene.draw(backend, &mut queue, view.eye())?;
    queue.flush(&view, backend)?;
    backend.end_frame()?;
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
    let mut game = build_scene(&mut backend, install, mode, options.double_sided)?;
    if options.fly {
        game.on_foot = false;
    }
    let fov = config.camera.fov;
    let aspect = w as f32 / h as f32;
    let tick = TICK.as_secs_f32();
    // Characters a little way into their clip, mid-stride when moving.
    game.scene.tick(0.4);
    render(&mut backend, &mut game, fov, aspect)?;
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
        render(&mut backend, &mut game, fov, aspect)?;
    }
    if options.drive && game.on_foot && game.scene.player.is_some() {
        drive_for_screenshot(&mut backend, &mut game, fov, aspect)?;
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
        for i in 0..180 {
            game.step(intent, tick);
            if i % 30 == 0 {
                // Keep streaming around her as she goes.
                render(&mut backend, &mut game, fov, aspect)?;
            }
        }
        if let (Some(p), Some(from)) = (&game.scene.player, from) {
            eprintln!(
                "player: {:?} from {from:.2} to {:.2}, grounded {}",
                gait, p.character.position, p.character.grounded
            );
        }
    }
    render(&mut backend, &mut game, fov, aspect)?;
    let rgba = backend.read_pixels()?;
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    ppm.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(out, ppm).with_context(|| format!("writing {}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

/// Waits for the parked vehicles, gets in the nearest and drives it
/// forward for four seconds, printing how it went.
fn drive_for_screenshot(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
) -> anyhow::Result<()> {
    let tick = TICK.as_secs_f32();
    let started = Instant::now();
    loop {
        game.step(Intent::default(), tick);
        render(backend, game, fov, aspect)?;
        let parked = game.scene.vehicles.iter().all(|v| v.id.is_some());
        if (!game.scene.vehicles_loading() && parked)
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
    if !game.enter_nearest(f32::INFINITY) {
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
    let from = game.driving().map(|(at, _)| at);
    let throttle = Intent {
        movement: m2_math::glam::Vec2::Y,
        ..Intent::default()
    };
    for i in 0..240 {
        game.step(throttle, tick);
        if i % 30 == 0 {
            render(backend, game, fov, aspect)?;
        }
    }
    if let (Some(from), Some((to, speed))) = (from, game.driving()) {
        eprintln!(
            "drive: {label} from {from:.2} to {to:.2} ({:.1} m), {speed:.1} m/s",
            (to - from).length()
        );
    }
    Ok(())
}

struct Running {
    window: Arc<Window>,
    backend: WgpuBackend,
    game: Game,
    audio: sound::GameAudio,
    input: Input,
    mouse_look: bool,
    /// Keyboard input is ignored while another window has focus.
    focused: bool,
    culling: bool,
    last: Instant,
    lag: Duration,
    fps: (Instant, u32),
    title: String,
}

struct App {
    config: Config,
    install: Option<Install>,
    mode: Mode,
    /// Start with back-face culling off.
    double_sided: bool,
    /// Start flying rather than on foot.
    fly: bool,
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
        )?;
        if self.fly {
            game.on_foot = false;
        }
        let focused = window.has_focus();
        Ok(Running {
            title: format!(
                "Mercenaries 2 (alpha) - {} on {}",
                adapter.name, adapter.backend
            ),
            window,
            backend,
            game,
            audio: sound::GameAudio::start(self.install.as_ref()),
            input: Input::default(),
            mouse_look: false,
            focused,
            culling: !self.double_sided,
            last: Instant::now(),
            lag: Duration::ZERO,
            fps: (Instant::now(), 0),
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

    /// Advances the simulation by whole ticks and draws a frame.
    fn frame(&mut self, config: &Config, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let now = Instant::now();
        self.lag += (now - self.last).min(MAX_FRAME);
        self.last = now;
        while self.lag >= TICK {
            self.lag -= TICK;
            self.tick(config, event_loop);
        }
        let (w, h) = self.backend.size();
        render(
            &mut self.backend,
            &mut self.game,
            config.camera.fov,
            w as f32 / h as f32,
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

    fn tick(&mut self, config: &Config, event_loop: &ActiveEventLoop) {
        if self.input.take_pressed(Action::Quit) {
            if self.mouse_look {
                self.set_mouse_look(false);
            } else {
                event_loop.exit();
            }
        }
        if self.input.take_pressed(Action::ToggleMouse) {
            self.set_mouse_look(!self.mouse_look);
        }
        if self.input.take_pressed(Action::Fullscreen) {
            let full = self.window.fullscreen().is_none();
            self.window
                .set_fullscreen(full.then_some(Fullscreen::Borderless(None)));
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
        let (ear, facing) = self.game.listener();
        self.audio.update(ear, facing);
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
                button: winit::event::MouseButton::Right,
                ..
            } if !state.mouse_look => state.set_mouse_look(true),
            WindowEvent::RedrawRequested => {
                if let Err(e) = state.frame(&self.config, event_loop) {
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
