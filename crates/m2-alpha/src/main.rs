//! `m2-alpha`: the native game, playing from the player's own install.
//!
//! ```text
//! m2-alpha [--install <dir>] [--model <hash>]... [--config <file>]
//!          [--size <w>x<h>] [--vsync on|off] [--fullscreen]
//!          [--screenshot <out.ppm>] [--write-config]
//! ```
//!
//! Settings come from `m2-alpha.toml` (in the working directory or beside
//! the executable) and the command line overrides them. `--write-config`
//! prints a settings file with every option at its current value.
//! `--screenshot` renders one frame without opening a window, for checking
//! a machine's GPU setup.
//!
//! Right now this opens a window with a free-flying camera over a ground
//! grid and the models named with `--model` (Jennifer by default) read from
//! the install. Tab captures the mouse, WASD/Space/C fly, Shift speeds up,
//! F11 toggles fullscreen, F3 toggles back-face culling, Escape quits.

mod camera;
mod config;
mod input;
mod scene;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, bail};
use m2_engine::Install;
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
use input::{Action, Input};
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
    let models = if options.models.is_empty() {
        vec![scene::DEFAULT_MODEL]
    } else {
        options.models.clone()
    };

    if let Some(out) = &options.screenshot {
        return screenshot(&config, install.as_ref(), &models, out);
    }

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App {
        config,
        install,
        models,
        state: None,
        error: None,
    };
    event_loop.run_app(&mut app)?;
    match app.error {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Builds the scene on `backend` and points a camera at it.
fn build_scene(
    backend: &mut dyn RenderBackend,
    install: Option<&Install>,
    models: &[u32],
) -> anyhow::Result<(Scene, FlyCamera)> {
    let mut scene = Scene::new(backend)?;
    if let Some(install) = install {
        scene.open(install)?;
        if let Err(e) = scene.load_models(backend, models) {
            eprintln!("could not load models: {e:#}");
        }
    }
    let (eye, target) = scene.focus;
    Ok((scene, FlyCamera::looking_at(eye, target)))
}

fn render(
    backend: &mut dyn RenderBackend,
    scene: &Scene,
    camera: &FlyCamera,
    fov: f32,
    aspect: f32,
) -> anyhow::Result<()> {
    let view = camera.view(fov, aspect);
    let mut queue = DrawQueue::new();
    scene.draw(&mut queue);
    backend.begin_frame(&view)?;
    queue.flush(&view, backend)?;
    backend.end_frame()?;
    Ok(())
}

/// Renders one frame offscreen and writes it as a binary PPM.
fn screenshot(
    config: &Config,
    install: Option<&Install>,
    models: &[u32],
    out: &std::path::Path,
) -> anyhow::Result<()> {
    let (w, h) = (config.window.width, config.window.height);
    let mut backend = WgpuBackend::headless(w, h)?;
    eprintln!("adapter: {:?}", backend.adapter_summary());
    let (scene, camera) = build_scene(&mut backend, install, models)?;
    render(
        &mut backend,
        &scene,
        &camera,
        config.camera.fov,
        w as f32 / h as f32,
    )?;
    let rgba = backend.read_pixels()?;
    let mut ppm = format!("P6\n{w} {h}\n255\n").into_bytes();
    ppm.extend(rgba.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]));
    std::fs::write(out, ppm).with_context(|| format!("writing {}", out.display()))?;
    eprintln!("wrote {}", out.display());
    Ok(())
}

struct Running {
    window: Arc<Window>,
    backend: WgpuBackend,
    scene: Scene,
    camera: FlyCamera,
    input: Input,
    mouse_look: bool,
    culling: bool,
    last: Instant,
    lag: Duration,
    fps: (Instant, u32),
    title: String,
}

struct App {
    config: Config,
    install: Option<Install>,
    models: Vec<u32>,
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
        let (scene, camera) = build_scene(&mut backend, self.install.as_ref(), &self.models)?;
        Ok(Running {
            title: format!(
                "Mercenaries 2 (alpha) - {} on {}",
                adapter.name, adapter.backend
            ),
            window,
            backend,
            scene,
            camera,
            input: Input::default(),
            mouse_look: false,
            culling: true,
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
            &self.scene,
            &self.camera,
            config.camera.fov,
            w as f32 / h as f32,
        )?;
        self.fps.1 += 1;
        let elapsed = now - self.fps.0;
        if elapsed >= Duration::from_secs(1) {
            let fps = f64::from(self.fps.1) / elapsed.as_secs_f64();
            let p = self.camera.position;
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
        if self.input.take_pressed(Action::ToggleCulling) {
            self.culling = !self.culling;
            if let Err(e) = self
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
        self.camera.update(
            &mut self.input,
            &config.camera,
            self.mouse_look,
            TICK.as_secs_f32(),
        );
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
            WindowEvent::Focused(false) => {
                state.input.clear();
                state.set_mouse_look(false);
            }
            WindowEvent::KeyboardInput { event, .. } => {
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
