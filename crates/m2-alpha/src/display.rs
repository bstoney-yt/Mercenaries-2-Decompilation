//! The window on the monitor: windowed, borderless fullscreen or
//! exclusive fullscreen ([`DisplayMode`]), the modes the video screens
//! list, and putting the window into the one the player kept
//! ([`crate::pause::Video`]).
//!
//! - Windowed: a framed window, `[window] width` x `height` (no larger
//!   than the monitor).
//! - Borderless: fullscreen over the monitor at its own (desktop) mode;
//!   nothing on the monitor changes, so switching is quick.
//! - Exclusive: fullscreen with the monitor switched to `width` x `height`
//!   at `refresh` hertz (0: its own rate). A size or rate it doesn't offer
//!   falls back to that size at the nearest slower rate, then to the
//!   monitor's own mode, then to borderless; the log says which. On
//!   Windows this is winit's display-mode change under a topmost window
//!   (the swapchain stays wgpu's flip model, which the compositor hands
//!   the screen to directly); the monitor goes back to its own mode on
//!   leaving it and when the process ends.
//!
//! F11 and Alt+Enter (the PC convention) toggle between a window and the
//! last fullscreen kind used (borderless unless the settings or the menus
//! chose exclusive), keeping the window's size for coming back
//! ([`Display`]). A menu kept rate is fullscreen at that mode: exclusive,
//! or, at the monitor's own size and rate, the fullscreen kind in use
//! (borderless from a window).
//!
//! All of it is invented: the game's executable, which did this on the
//! PC, isn't read; the movie only shows the texts it is given (see
//! [`crate::pause`]).

use winit::dpi::PhysicalSize;
use winit::monitor::{MonitorHandle, VideoModeHandle};
use winit::window::{Fullscreen, Window};

pub use crate::config::DisplayMode;
use crate::config::WindowConfig;
use crate::pause::{Mode, Video};

/// Millihertz to whole hertz, rounded (59.94 Hz is 60).
fn hz(millihertz: u32) -> u32 {
    (millihertz + 500) / 1000
}

/// The monitor's own refresh rate in hertz (60 if it doesn't say).
pub fn own_rate(monitor: Option<&MonitorHandle>) -> u32 {
    monitor
        .and_then(MonitorHandle::refresh_rate_millihertz)
        .map_or(60, hz)
}

/// The modes `monitor` offers, each size and rate once.
pub fn modes(monitor: Option<&MonitorHandle>) -> Vec<Mode> {
    let mut modes: Vec<Mode> = monitor
        .map(|m| {
            m.video_modes()
                .map(|v| Mode {
                    width: v.size().width,
                    height: v.size().height,
                    hz: hz(v.refresh_rate_millihertz()),
                })
                .collect()
        })
        .unwrap_or_default();
    modes.sort_unstable();
    modes.dedup();
    modes
}

/// The primary monitor's modes, without a window: for the offscreen
/// pictures (`--pause`), which have no event loop of their own. Only once
/// a run (winit allows one event loop).
pub fn primary_modes() -> Vec<Mode> {
    use winit::application::ApplicationHandler;
    use winit::event::WindowEvent;
    use winit::event_loop::{ActiveEventLoop, EventLoop};
    use winit::window::WindowId;

    #[derive(Default)]
    struct Probe(Vec<Mode>);
    impl ApplicationHandler for Probe {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.0 = modes(event_loop.primary_monitor().as_ref());
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let mut probe = Probe::default();
    if let Ok(event_loop) = EventLoop::new() {
        let _ = event_loop.run_app(&mut probe);
    }
    probe.0
}

/// A mode a monitor offers, as numbers to choose among.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Offer {
    width: u32,
    height: u32,
    hz: u32,
    depth: u16,
}

/// How [`pick`] found the mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Found {
    /// The size and rate asked for.
    Asked,
    /// The size, at another rate.
    OtherRate,
    /// The monitor's own mode (the size isn't offered).
    Own,
}

/// Which of `offers` exclusive fullscreen at `want` (width, height,
/// hertz) takes, on a monitor whose own mode is `own`: a zero size or
/// rate is the monitor's own. The size at that rate, else at the fastest
/// slower rate (else its slowest), else the monitor's own mode; the
/// deepest color of each. `None`: not even that (borderless, then).
fn pick(offers: &[Offer], want: (u32, u32, u32), own: (u32, u32, u32)) -> Option<(usize, Found)> {
    let (w, h) = if want.0 == 0 || want.1 == 0 {
        (own.0, own.1)
    } else {
        (want.0, want.1)
    };
    let rate = if want.2 == 0 { own.2 } else { want.2 };
    let best = |keep: &dyn Fn(&Offer) -> bool, key: &dyn Fn(&Offer) -> (i64, u16)| {
        offers
            .iter()
            .enumerate()
            .filter(|(_, o)| keep(o))
            .max_by_key(|(_, o)| key(o))
            .map(|(i, _)| i)
    };
    let sized = |o: &Offer| (o.width, o.height) == (w, h);
    if let Some(i) = best(&|o| sized(o) && o.hz == rate, &|o| (0, o.depth)) {
        return Some((i, Found::Asked));
    }
    // Slower rates first, the fastest of them; then the slowest faster one.
    let near = |o: &Offer| {
        if o.hz <= rate {
            (i64::from(o.hz), o.depth)
        } else {
            (-i64::from(o.hz) - 1_000_000, o.depth)
        }
    };
    if let Some(i) = best(&sized, &near) {
        return Some((i, Found::OtherRate));
    }
    best(&|o| (o.width, o.height, o.hz) == own, &|o| (0, o.depth)).map(|i| (i, Found::Own))
}

/// The monitor's own mode: its size and rate now.
fn own_mode(m: &MonitorHandle) -> (u32, u32, u32) {
    let s = m.size();
    (s.width, s.height, own_rate(Some(m)))
}

/// The fullscreen `mode` asks for on `monitor` (exclusive at `size` and
/// `rate`, see [`pick`]), `None` windowed; and what it is, for the log.
pub fn fullscreen_for(
    monitor: Option<MonitorHandle>,
    mode: DisplayMode,
    size: (u32, u32),
    rate: u32,
) -> (Option<Fullscreen>, String) {
    match mode {
        DisplayMode::Windowed => (None, "windowed".into()),
        DisplayMode::Borderless => (
            Some(Fullscreen::Borderless(monitor)),
            "borderless fullscreen at the monitor's own mode".into(),
        ),
        DisplayMode::Exclusive => {
            let Some(m) = monitor else {
                return (
                    Some(Fullscreen::Borderless(None)),
                    "borderless fullscreen (no monitor to change the mode of)".into(),
                );
            };
            let handles: Vec<VideoModeHandle> = m.video_modes().collect();
            let offers: Vec<Offer> = handles
                .iter()
                .map(|v| Offer {
                    width: v.size().width,
                    height: v.size().height,
                    hz: hz(v.refresh_rate_millihertz()),
                    depth: v.bit_depth(),
                })
                .collect();
            match pick(&offers, (size.0, size.1, rate), own_mode(&m)) {
                Some((i, found)) => {
                    let o = offers[i];
                    let how = match found {
                        Found::Asked => String::new(),
                        Found::OtherRate => format!(" ({rate} Hz isn't offered at that size)"),
                        Found::Own => format!(
                            " ({}x{} isn't offered: the monitor's own mode)",
                            size.0, size.1
                        ),
                    };
                    (
                        Some(Fullscreen::Exclusive(handles[i].clone())),
                        format!(
                            "exclusive fullscreen at {}x{} {} Hz{how}",
                            o.width, o.height, o.hz
                        ),
                    )
                }
                None => (
                    Some(Fullscreen::Borderless(Some(m))),
                    "borderless fullscreen (the monitor lists no mode to switch to)".into(),
                ),
            }
        }
    }
}

/// The fullscreen kind `window` is in.
pub fn mode(window: &Window) -> DisplayMode {
    match window.fullscreen() {
        Some(Fullscreen::Exclusive(_)) => DisplayMode::Exclusive,
        Some(Fullscreen::Borderless(_)) => DisplayMode::Borderless,
        None => DisplayMode::Windowed,
    }
}

/// How `window` is shown now.
pub fn current(window: &Window) -> Video {
    let size = window.inner_size();
    let fullscreen = match window.fullscreen() {
        Some(Fullscreen::Exclusive(mode)) => Some(hz(mode.refresh_rate_millihertz())),
        Some(Fullscreen::Borderless(_)) => Some(own_rate(window.current_monitor().as_ref())),
        None => None,
    };
    Video {
        width: size.width,
        height: size.height,
        fullscreen,
    }
}

/// A window of `size` that fits on `monitor` with its frame: no more
/// than nine tenths of the monitor's own mode either way (scaled down
/// whole, keeping its shape), nor smaller than 320 x 200.
pub fn fit(size: (u32, u32), monitor: Option<&MonitorHandle>) -> PhysicalSize<u32> {
    let room = monitor.map(|m| (m.size().width * 9 / 10, m.size().height * 9 / 10));
    let (w, h) = fit_in(size, room);
    PhysicalSize::new(w, h)
}

fn fit_in((w, h): (u32, u32), room: Option<(u32, u32)>) -> (u32, u32) {
    let (w, h) = (w.max(320), h.max(200));
    match room {
        Some((rw, rh)) if rw > 0 && rh > 0 && (w > rw || h > rh) => {
            let scale = (f64::from(rw) / f64::from(w)).min(f64::from(rh) / f64::from(h));
            (
                ((f64::from(w) * scale) as u32).max(1),
                ((f64::from(h) * scale) as u32).max(1),
            )
        }
        _ => (w, h),
    }
}

/// The mode a kept [`Video`] means, from `now` (the window's): a window;
/// fullscreen at the monitor's own size and rate (`own`) is the kind in
/// use, borderless from a window; at any other mode, exclusive.
fn mode_of_video(v: Video, own: (u32, u32, u32), now: DisplayMode) -> DisplayMode {
    match v.fullscreen {
        None => DisplayMode::Windowed,
        Some(rate) if (v.width, v.height, rate) == own => match now {
            DisplayMode::Windowed => DisplayMode::Borderless,
            full => full,
        },
        Some(_) => DisplayMode::Exclusive,
    }
}

/// Puts `window` into the `video` a menu kept, and the settings with it;
/// what it did, for the log. The swapchain follows the `Resized` that
/// comes of it.
pub fn apply(window: &Window, video: Video, config: &mut WindowConfig) -> String {
    let monitor = window.current_monitor();
    let own = monitor.as_ref().map_or((0, 0, 0), own_mode);
    let mode = mode_of_video(video, own, self::mode(window));
    let rate = video.fullscreen.unwrap_or(0);
    config.mode = mode;
    (config.width, config.height) = (video.width, video.height);
    config.refresh = if mode == DisplayMode::Exclusive {
        rate
    } else {
        0
    };
    show(window, mode, (video.width, video.height), rate).0
}

/// Puts `window` into `mode` (a window of `size`, or exclusive at `size`
/// and `rate`); what it did, for the log, and the window's size asked.
fn show(
    window: &Window,
    mode: DisplayMode,
    size: (u32, u32),
    rate: u32,
) -> (String, Option<(u32, u32)>) {
    let monitor = window.current_monitor();
    let (full, how) = fullscreen_for(monitor.clone(), mode, size, rate);
    match full {
        Some(full) => {
            set_pending(None);
            window.set_fullscreen(Some(full));
            (how, None)
        }
        None => {
            // Out of exclusive, the monitor is still at its mode, not
            // the desktop's a window fits on: the size was fitted when
            // it was a window, or at the start.
            let size = if self::mode(window) == DisplayMode::Exclusive {
                size
            } else {
                let s = fit(size, monitor.as_ref());
                (s.width, s.height)
            };
            window.set_fullscreen(None);
            window.set_maximized(false);
            let _ = window.request_inner_size(PhysicalSize::new(size.0, size.1));
            set_pending(Some((size, 3)));
            (format!("a {}x{} window", size.0, size.1), Some(size))
        }
    }
}

/// Back to a window: the size asked for, and how many more times to ask.
/// Leaving fullscreen, Windows puts back the place the window had before
/// it, which can come after our size and undo it (a window created
/// fullscreen had no place but its first). One window a process.
static PENDING: std::sync::Mutex<Option<((u32, u32), u8)>> = std::sync::Mutex::new(None);

fn set_pending(p: Option<((u32, u32), u8)>) {
    if let Ok(mut pending) = PENDING.lock() {
        *pending = p;
    }
}

/// What `window` is on, for the log: its mode, its size and the
/// monitor's mode now.
pub fn describe(window: &Window) -> String {
    let size = window.inner_size();
    let monitor = window.current_monitor();
    let (mw, mh, mhz) = monitor.as_ref().map_or((0, 0, 0), own_mode);
    format!(
        "{}, window {}x{}, monitor {mw}x{mh} {mhz} Hz",
        mode(window).text(),
        size.width,
        size.height
    )
}

/// What F11 and Alt+Enter go back and forth between.
#[derive(Clone, Copy, Debug)]
pub struct Display {
    /// The window's size to come back to (its place winit keeps).
    windowed: (u32, u32),
    /// The fullscreen kind to go to from a window.
    full: DisplayMode,
    /// Exclusive's size and rate (0: the monitor's own).
    exclusive: (u32, u32, u32),
}

impl Display {
    pub fn new(config: &WindowConfig) -> Self {
        let size = (config.width, config.height);
        Self {
            windowed: size,
            full: match config.mode {
                DisplayMode::Windowed => DisplayMode::Borderless,
                full => full,
            },
            exclusive: match config.mode {
                DisplayMode::Exclusive => (size.0, size.1, config.refresh),
                _ => (0, 0, 0),
            },
        }
    }

    /// The window was resized: as a plain window (not fullscreen,
    /// maximized or minimized), the size to come back to.
    pub fn resized(&mut self, window: &Window, size: PhysicalSize<u32>) {
        let plain = window.fullscreen().is_none()
            && !window.is_maximized()
            && window.is_minimized() != Some(true);
        // A `Resized` can carry a size the window has already left (on
        // the way in or out of fullscreen); the window's own is the one.
        let inner = window.inner_size();
        if !plain || size != inner || size.width == 0 || size.height == 0 {
            return;
        }
        let now = (size.width, size.height);
        let pending = PENDING.lock().ok().and_then(|p| *p);
        match pending {
            Some((asked, tries)) if asked != now && tries > 0 => {
                set_pending(Some((asked, tries - 1)));
                let _ = window.request_inner_size(PhysicalSize::new(asked.0, asked.1));
                return;
            }
            Some(_) => set_pending(None),
            None => {}
        }
        // Our own size fitted to a small monitor isn't the player's: the
        // size asked for stays, to fit whole on a larger one next time.
        let fitted = fit(self.windowed, window.current_monitor().as_ref());
        if now != (fitted.width, fitted.height) {
            self.windowed = now;
        }
    }

    /// F11 or Alt+Enter: a window to the fullscreen kind last used, or
    /// fullscreen to a window, and the settings with it; what it did.
    pub fn toggle(&mut self, window: &Window, config: &mut WindowConfig) -> String {
        let now = window.fullscreen();
        let to = match &now {
            None => self.full,
            Some(Fullscreen::Borderless(_)) => {
                self.full = DisplayMode::Borderless;
                DisplayMode::Windowed
            }
            Some(Fullscreen::Exclusive(m)) => {
                self.full = DisplayMode::Exclusive;
                self.exclusive = (
                    m.size().width,
                    m.size().height,
                    hz(m.refresh_rate_millihertz()),
                );
                DisplayMode::Windowed
            }
        };
        self.set(window, to, config)
    }

    /// Puts `window` into `mode` (the menus' choice of the three), and the
    /// settings with it: a window at the size it had, exclusive at the
    /// mode last used (the monitor's own at first); what it did.
    pub fn set(&mut self, window: &Window, mode: DisplayMode, config: &mut WindowConfig) -> String {
        if mode.is_fullscreen() {
            self.full = mode;
        }
        if let Some(Fullscreen::Exclusive(m)) = window.fullscreen() {
            self.exclusive = (
                m.size().width,
                m.size().height,
                hz(m.refresh_rate_millihertz()),
            );
        }
        let (size, rate) = match mode {
            DisplayMode::Windowed => (self.windowed, 0),
            DisplayMode::Borderless => (self.windowed, 0),
            DisplayMode::Exclusive => ((self.exclusive.0, self.exclusive.1), self.exclusive.2),
        };
        let (how, _) = show(window, mode, size, rate);
        config.mode = mode;
        if mode == DisplayMode::Exclusive {
            // The mode it took (a size it asked for may not be offered).
            if let Some(Fullscreen::Exclusive(m)) = window.fullscreen() {
                (config.width, config.height) = (m.size().width, m.size().height);
                config.refresh = hz(m.refresh_rate_millihertz());
                return how;
            }
        }
        (config.width, config.height) = size;
        config.refresh = rate;
        how
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offer(width: u32, height: u32, hz: u32, depth: u16) -> Offer {
        Offer {
            width,
            height,
            hz,
            depth,
        }
    }

    const OWN: (u32, u32, u32) = (1920, 1080, 60);

    fn offers() -> Vec<Offer> {
        vec![
            offer(1920, 1080, 60, 32),
            offer(1920, 1080, 60, 16),
            offer(1920, 1080, 144, 32),
            offer(1280, 720, 60, 32),
            offer(1280, 720, 120, 32),
            offer(1024, 768, 75, 32),
        ]
    }

    #[test]
    fn rates_round_to_whole_hertz() {
        assert_eq!(hz(59_940), 60);
        assert_eq!(hz(164_999), 165);
        assert_eq!(hz(143_856), 144);
        assert_eq!(own_rate(None), 60);
        assert!(modes(None).is_empty());
    }

    #[test]
    fn exclusive_takes_the_mode_asked_for_at_its_deepest_color() {
        let o = offers();
        assert_eq!(pick(&o, (1920, 1080, 144), OWN), Some((2, Found::Asked)));
        assert_eq!(pick(&o, (1280, 720, 60), OWN), Some((3, Found::Asked)));
        // Zero is the monitor's own: size, rate or both; 32 bits over 16.
        assert_eq!(pick(&o, (0, 0, 0), OWN), Some((0, Found::Asked)));
        assert_eq!(pick(&o, (1920, 1080, 0), OWN), Some((0, Found::Asked)));
        assert_eq!(pick(&o, (0, 0, 144), OWN), Some((2, Found::Asked)));
    }

    #[test]
    fn exclusive_falls_back_to_a_near_rate_then_the_monitors_own() {
        let o = offers();
        // 100 Hz at 1280x720: the fastest slower rate, 60.
        assert_eq!(pick(&o, (1280, 720, 100), OWN), Some((3, Found::OtherRate)));
        // 50 Hz: nothing slower, so the slowest faster one.
        assert_eq!(pick(&o, (1024, 768, 50), OWN), Some((5, Found::OtherRate)));
        assert_eq!(
            pick(&o, (1920, 1080, 240), OWN),
            Some((2, Found::OtherRate))
        );
        // A size it doesn't offer: the monitor's own mode.
        assert_eq!(pick(&o, (1600, 900, 60), OWN), Some((0, Found::Own)));
        // Nothing at all: borderless.
        assert_eq!(pick(&[], (1600, 900, 60), OWN), None);
        assert_eq!(pick(&o[3..], (1600, 900, 60), OWN), None);
    }

    #[test]
    fn a_window_fits_on_the_monitor_with_its_frame() {
        let room = Some((2304, 1296)); // nine tenths of 2560 x 1440
        assert_eq!(fit_in((1280, 720), room), (1280, 720));
        assert_eq!(fit_in((2304, 1296), room), (2304, 1296));
        // Too big: scaled down whole, its shape kept.
        assert_eq!(fit_in((2560, 1440), room), (2304, 1296));
        assert_eq!(fit_in((4000, 3000), room), (1728, 1296));
        assert_eq!(fit_in((10, 10), room), (320, 200));
        assert_eq!(fit_in((4000, 3000), None), (4000, 3000));
    }

    #[test]
    fn a_kept_video_means_a_mode() {
        let window = Video {
            width: 1280,
            height: 720,
            fullscreen: None,
        };
        let own = Video {
            width: 1920,
            height: 1080,
            fullscreen: Some(60),
        };
        let other = Video {
            width: 1280,
            height: 720,
            fullscreen: Some(60),
        };
        use DisplayMode::*;
        for now in DisplayMode::ALL {
            assert_eq!(mode_of_video(window, OWN, now), Windowed);
            assert_eq!(mode_of_video(other, OWN, now), Exclusive);
        }
        assert_eq!(mode_of_video(own, OWN, Windowed), Borderless);
        assert_eq!(mode_of_video(own, OWN, Borderless), Borderless);
        assert_eq!(mode_of_video(own, OWN, Exclusive), Exclusive);
    }

    #[test]
    fn display_starts_from_the_settings() {
        let mut config = WindowConfig::default();
        let d = Display::new(&config);
        assert_eq!(
            (d.windowed, d.full, d.exclusive),
            ((1600, 900), DisplayMode::Borderless, (0, 0, 0))
        );
        config.mode = DisplayMode::Exclusive;
        (config.width, config.height, config.refresh) = (1280, 720, 120);
        let d = Display::new(&config);
        assert_eq!(
            (d.full, d.exclusive),
            (DisplayMode::Exclusive, (1280, 720, 120))
        );
    }
}
