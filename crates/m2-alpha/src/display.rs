//! The window on the monitor: the modes the pause menu's video screen
//! lists, and putting the window into the one the player kept
//! ([`crate::pause::Video`]).
//!
//! A kept rate is fullscreen at that mode: borderless when it is the
//! monitor's own (its desktop size and rate), exclusive otherwise; a kept
//! `Windowed` is a window of that size. F11 and Alt+Enter (the PC
//! convention) toggle borderless fullscreen at the monitor's own mode.
//! All of it is invented: the game's executable, which did this on the PC,
//! isn't read; the movie only shows the texts it is given (see
//! [`crate::pause`]).

use winit::dpi::PhysicalSize;
use winit::monitor::{MonitorHandle, VideoModeHandle};
use winit::window::{Fullscreen, Window};

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

/// The monitor's mode of that size and rate, its deepest color.
fn mode_of(monitor: &MonitorHandle, width: u32, height: u32, rate: u32) -> Option<VideoModeHandle> {
    monitor
        .video_modes()
        .filter(|v| {
            v.size() == PhysicalSize::new(width, height) && hz(v.refresh_rate_millihertz()) == rate
        })
        .max_by_key(VideoModeHandle::bit_depth)
}

/// Fullscreen at `width` x `height` and `rate` hertz on `monitor`:
/// borderless at its own mode (or rate 0, or no such mode), exclusive at
/// any other.
pub fn fullscreen_at(
    monitor: Option<MonitorHandle>,
    width: u32,
    height: u32,
    rate: u32,
) -> Fullscreen {
    let Some(m) = monitor else {
        return Fullscreen::Borderless(None);
    };
    let own = m.size() == PhysicalSize::new(width, height) && own_rate(Some(&m)) == rate;
    if rate == 0 || own {
        return Fullscreen::Borderless(Some(m));
    }
    match mode_of(&m, width, height, rate) {
        Some(mode) => Fullscreen::Exclusive(mode),
        None => Fullscreen::Borderless(Some(m)),
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

/// Puts `window` into `video`; what it did, for the log. The swapchain
/// follows the `Resized` that comes of it.
pub fn apply(window: &Window, video: Video) -> String {
    match video.fullscreen {
        Some(rate) => {
            let full = fullscreen_at(window.current_monitor(), video.width, video.height, rate);
            let how = match &full {
                Fullscreen::Exclusive(m) => format!(
                    "exclusive fullscreen at {}x{} {} Hz",
                    m.size().width,
                    m.size().height,
                    hz(m.refresh_rate_millihertz())
                ),
                Fullscreen::Borderless(_) => {
                    "borderless fullscreen at the monitor's own mode".into()
                }
            };
            window.set_fullscreen(Some(full));
            how
        }
        None => {
            window.set_fullscreen(None);
            window.set_maximized(false);
            let _ = window.request_inner_size(PhysicalSize::new(video.width, video.height));
            format!("a {}x{} window", video.width, video.height)
        }
    }
}

/// F11 or Alt+Enter: borderless fullscreen at the monitor's own mode, or
/// back to a window.
pub fn toggle(window: &Window) {
    let full = window.fullscreen().is_none();
    window.set_fullscreen(full.then_some(Fullscreen::Borderless(None)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_round_to_whole_hertz() {
        assert_eq!(hz(59_940), 60);
        assert_eq!(hz(164_999), 165);
        assert_eq!(hz(143_856), 144);
        assert_eq!(own_rate(None), 60);
        assert!(modes(None).is_empty());
    }
}
