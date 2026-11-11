//! A helicopter's winch: its `Winch` component and the state of its
//! cable (the 360's `PhysicsActorWinch`, 0x823C5BA8, one a helicopter in
//! the store 0x83657220).
//!
//! From the 360 code:
//! - **The component** (`Winch`, registered 0x823AAF28, fields and
//!   defaults 0x823EC538): `Length` 5, `WinchBone`, `MagnetRadius` 1,
//!   `Deployed`, `CameraZoomOffset` -4, `CameraZoomOffsetAttached` 0,
//!   `VertFocusOffset` -7, `VertFocusOffsetAttached` -6, `CameraYawOffset`
//!   -35, `CameraPitchOffset` -14.324, `RetractAutodropWhenLengthShorter`
//!   10 000 000.
//! - **The actor** (built by 0x823FE8B0 through 0x821952E8): the cable is
//!   a chain of links 2 x 0.08 m long (+0x150); at most
//!   `max(7, trunc(Length x 6.25 + 1))` of them (+0x160); it lets its
//!   cargo drop when retracted to `max(4, trunc(autodrop x 6.25))` links
//!   or fewer (+0x200). It starts deployed if `Deployed` is set (+0x140).
//! - **Its states** (+0x140; `Object.GetWinchState`'s names, 0x823336E8):
//!   0 retracted, 1 deployed, 2 deploying, 3 retracting.
//! - **Paying out and reeling in** (0x82197FC0, each step of the update
//!   0x82195780): deploying at 6 m/s, retracting at 3 m/s, in links of
//!   0.16 m; +0x16C is how far the end link is out (deploying, the part
//!   still to go; retracting, the part out). Links are added by 0x821962B8
//!   (two at once for the first) and taken by 0x82197360. Deploying, once
//!   all links are out and the end one fully (0x82196AC0) it is deployed;
//!   retracting, with no link left (0x82197360) it is retracted. Deployed
//!   with no links it pays them all out at once; retracted with links, it
//!   takes them all in.
//! - **Its buttons** (0x82198118, actions 0xA4E4316F and 0xB422F25E, each
//!   on a press above 0): the first reels in a deployed or deploying
//!   winch. The second pays out a retracted one, or one retracting that
//!   isn't stretched; else, with cargo on and no more links out than the
//!   autodrop count, drops the cargo; else reels in a deployed or
//!   deploying one.
//! - **Stretched** (0x82198308): deployed, the cable is stretched when
//!   any two links next to each other are 4 x their length squared apart
//!   or more, or on average 1.21 x; stretched over 0.2 s, it drops its
//!   cargo, or with none reels in.
//! - **Cargo** (the update): reeling in with cargo on and no more links
//!   out than the autodrop count, or retracted, the cargo drops.
//!
//! - **Her controls** (0x824AFFD8, for a helicopter she flies with a
//!   winch): asking for a state other than the winch's presses the second
//!   button when it is retracted and asked to deploy, or deployed and asked
//!   to retract; deployed, holding the hook button with a target not
//!   already on, it hooks the target, first putting it under the hook if
//!   it is over a metre away (velocities zeroed); letting go of the
//!   button drops the cargo ([`player_step`]).
//!
//! Not ported here: how her controls pick the target (+0x19, filled
//! elsewhere), the links' bodies and their physics (0x821962B8,
//! 0x82196BB8, read through VMX), the attach and the hook (0x82195C30,
//! 0x82196238), the sounds each change plays (0x82174F88), the ground
//! check that stops a winch deploying near the ground (0x82167400).

use std::collections::HashMap;

use m2_assets::layer::Layer;
use m2_assets::name_hash;

use crate::tuning::Table;

/// A link of the cable, metres (2 x 0.08, +0x150).
pub const LINK: f32 = 0.16;
/// Paying out, m/s (0x820B7B50).
pub const DEPLOY_SPEED: f32 = 6.0;
/// Reeling in, m/s (0x820C7A20).
pub const RETRACT_SPEED: f32 = 3.0;
/// How long the cable may be stretched before it gives (0x820B8054), s.
pub const STRETCH_GIVES: f32 = 0.2;

/// The `Winch` component's fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WinchParams {
    pub length: f32,
    pub bone: Option<u32>,
    pub magnet_radius: f32,
    pub deployed: bool,
    pub camera_zoom_offset: f32,
    pub camera_zoom_offset_attached: f32,
    pub vert_focus_offset: f32,
    pub vert_focus_offset_attached: f32,
    pub camera_yaw_offset: f32,
    pub camera_pitch_offset: f32,
    pub retract_autodrop_when_length_shorter: f32,
}

impl Default for WinchParams {
    /// 0x823EC538's defaults.
    fn default() -> Self {
        Self {
            length: 5.0,
            bone: None,
            magnet_radius: 1.0,
            deployed: false,
            camera_zoom_offset: -4.0,
            camera_zoom_offset_attached: 0.0,
            vert_focus_offset: -7.0,
            vert_focus_offset_attached: -6.0,
            camera_yaw_offset: -35.0,
            camera_pitch_offset: -14.324,
            retract_autodrop_when_length_shorter: 10_000_000.0,
        }
    }
}

/// Every template's `Winch`, by template id.
pub fn read(db: &Layer<'_>) -> HashMap<u32, WinchParams> {
    let t = Table::new(db, "Winch");
    let h = name_hash;
    let d = WinchParams::default();
    t.templates()
        .filter_map(|id| {
            let r = *t.of_template(id).first()?;
            let f = |name: &str, dflt: f32| t.float(r, h(name)).unwrap_or(dflt);
            Some((
                id,
                WinchParams {
                    length: f("Length", d.length),
                    bone: t.word(r, h("WinchBone")).filter(|&b| b != 0),
                    magnet_radius: f("MagnetRadius", d.magnet_radius),
                    deployed: t.word(r, h("Deployed")).is_some_and(|b| b != 0),
                    camera_zoom_offset: f("CameraZoomOffset", d.camera_zoom_offset),
                    camera_zoom_offset_attached: f(
                        "CameraZoomOffsetAttached",
                        d.camera_zoom_offset_attached,
                    ),
                    vert_focus_offset: f("VertFocusOffset", d.vert_focus_offset),
                    vert_focus_offset_attached: f(
                        "VertFocusOffsetAttached",
                        d.vert_focus_offset_attached,
                    ),
                    camera_yaw_offset: f("CameraYawOffset", d.camera_yaw_offset),
                    camera_pitch_offset: f("CameraPitchOffset", d.camera_pitch_offset),
                    retract_autodrop_when_length_shorter: f(
                        "RetractAutodropWhenLengthShorter",
                        d.retract_autodrop_when_length_shorter,
                    ),
                },
            ))
        })
        .collect()
}

/// The winch's state (+0x140), as `Object.GetWinchState` names it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum WinchState {
    #[default]
    Retracted = 0,
    Deployed = 1,
    Deploying = 2,
    Retracting = 3,
}

impl WinchState {
    /// `Object.GetWinchState`'s name (0x823336E8).
    pub fn name(self) -> &'static str {
        match self {
            WinchState::Retracted => "retracted",
            WinchState::Deployed => "deployed",
            WinchState::Deploying => "deploying",
            WinchState::Retracting => "retracting",
        }
    }

    /// `Object.SetWinchState`'s argument (0x823337E8): one of the names.
    pub fn named(name: &str) -> Option<Self> {
        [
            WinchState::Retracted,
            WinchState::Deployed,
            WinchState::Deploying,
            WinchState::Retracting,
        ]
        .into_iter()
        .find(|s| s.name().eq_ignore_ascii_case(name))
    }
}

/// What a step or a press asks of the cable's bodies and its cargo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Change {
    /// Links to add at the hook end, and to take off.
    pub add: usize,
    pub take: usize,
    /// The cargo is let go.
    pub drop: bool,
}

/// A winch's cable state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Winch {
    pub state: WinchState,
    /// Links out (+0x50), at most `max_links`.
    pub links: usize,
    pub max_links: usize,
    /// Links out at or under which reeling in drops the cargo (+0x200).
    pub autodrop_links: usize,
    /// How far the end link is out (+0x16C).
    pub end: f32,
    /// How long the cable has been stretched (+0x1FC), s.
    pub stretched: f32,
    /// Cargo is on it (+0x170).
    pub cargo: bool,
}

impl Winch {
    /// 0x821952E8.
    pub fn new(p: &WinchParams) -> Self {
        Self {
            state: if p.deployed {
                WinchState::Deployed
            } else {
                WinchState::Retracted
            },
            links: 0,
            max_links: ((p.length * 6.25 + 1.0) as i32).max(7) as usize,
            autodrop_links: ((p.retract_autodrop_when_length_shorter * 6.25) as i32).max(4)
                as usize,
            end: 0.0,
            stretched: 0.0,
            cargo: false,
        }
    }

    fn add(&mut self, n: usize, c: &mut Change) {
        // 0x821962B8: no more than the rest; the first goes out as two.
        let mut n = n.min(self.max_links - self.links);
        if n == 0 {
            return;
        }
        if n < 2 && self.links == 0 {
            self.end = 0.0;
            n = 2;
        }
        self.links += n;
        c.add += n;
    }

    fn take(&mut self, n: usize, c: &mut Change) {
        // 0x82197360: with two or fewer left, all of them.
        let n = if self.links.saturating_sub(n) < 2 {
            self.links
        } else {
            n
        };
        if self.links - n <= self.autodrop_links && self.cargo {
            self.drop(c);
        }
        self.links -= n;
        c.take += n;
        if self.links == 0 {
            self.state = WinchState::Retracted;
            self.end = 0.0;
        }
    }

    fn drop(&mut self, c: &mut Change) {
        if self.cargo {
            self.cargo = false;
            c.drop = true;
        }
    }

    /// The first button (0xA4E4316F): reel in.
    pub fn press_retract(&mut self) {
        if matches!(self.state, WinchState::Deployed | WinchState::Deploying) {
            self.state = WinchState::Retracting;
        }
    }

    /// The second button (0xB422F25E): pay out, drop, or reel in.
    pub fn press_deploy(&mut self) -> Change {
        let mut c = Change::default();
        match self.state {
            WinchState::Retracted => self.state = WinchState::Deploying,
            WinchState::Retracting if self.stretched == 0.0 => {
                self.state = WinchState::Deploying;
            }
            _ if self.cargo && self.links <= self.autodrop_links => self.drop(&mut c),
            WinchState::Deployed | WinchState::Deploying => {
                self.state = WinchState::Retracting;
            }
            WinchState::Retracting => {}
        }
        c
    }

    /// One step of `dt` (0x82195780's state work and 0x82198308);
    /// `stretched`: the links are as far apart as 0x82198308 counts as
    /// stretched.
    pub fn step(&mut self, dt: f32, stretched: bool) -> Change {
        let mut c = Change::default();
        match self.state {
            WinchState::Retracted => {
                if self.links > 0 {
                    self.take(self.links, &mut c);
                }
                self.end = 0.0;
            }
            WinchState::Deployed => {
                if self.links == 0 {
                    self.add(self.max_links, &mut c);
                }
            }
            WinchState::Deploying | WinchState::Retracting => self.pay(dt, &mut c),
        }
        // The cargo drops reeling in at the autodrop length, or retracted.
        if self.cargo
            && (self.state == WinchState::Retracted
                || self.state == WinchState::Retracting && self.links <= self.autodrop_links)
        {
            self.drop(&mut c);
        }
        // 0x82198308.
        if self.state == WinchState::Deployed && self.links > 1 && stretched {
            self.stretched += dt;
        } else if self.state != WinchState::Retracting {
            // Not stretched, or retracted: it starts again.
            self.stretched = 0.0;
        }
        if self.stretched > STRETCH_GIVES {
            if self.cargo {
                self.drop(&mut c);
                self.stretched = 0.0;
            } else {
                self.state = WinchState::Retracting;
            }
        }
        c
    }

    /// 0x82197FC0.
    fn pay(&mut self, dt: f32, c: &mut Change) {
        let deploying = self.state == WinchState::Deploying;
        let rate = if deploying {
            DEPLOY_SPEED
        } else {
            RETRACT_SPEED
        };
        let left = if deploying { self.end } else { 1.0 - self.end };
        let go = rate * dt / LINK;
        if left > go {
            let rest = left - go;
            self.end = if deploying { rest } else { 1.0 - rest };
            self.settle();
            return;
        }
        if deploying && self.max_links == self.links {
            self.end = 0.0;
            self.settle();
            return;
        }
        let over = go - left;
        let mut n = over as usize;
        let part = over - n as f32;
        if part <= 0.0 {
            self.end = if deploying { 0.0 } else { 1.0 };
        } else {
            n += 1;
            self.end = if deploying { 1.0 - part } else { part };
        }
        if n == 0 {
            self.settle();
        } else if deploying {
            self.add(n, c);
        } else {
            self.take(n, c);
        }
    }

    /// 0x82196AC0: all out, deployed.
    fn settle(&mut self) {
        if self.state == WinchState::Deploying && self.end == 0.0 && self.links == self.max_links {
            self.state = WinchState::Deployed;
        }
    }

    /// How much cable is out, metres.
    pub fn length(&self) -> f32 {
        self.links as f32 * LINK
    }
}

/// The player's winch controls for a step, as her helicopter's input
/// has them (0x824AFFD8 reads its input's +0x17 to +0x19).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WinchPad {
    /// The winch is hers to work (+0x17 bit 7).
    pub on: bool,
    /// The state she asks for (+0x17 bits 0 and 1).
    pub want: u8,
    /// She holds the hook-on button (+0x18 bit 7).
    pub hold: bool,
}

/// What the player's controls ask of the winch this step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PlayerAsk {
    /// The second button's action (0xB422F25E, [`Winch::press_deploy`]).
    pub press: bool,
    /// Hook on her target (0x82195C30), first putting it under the hook
    /// when it is more than a metre from it.
    pub hook: bool,
    pub snap: bool,
    /// Let the cargo go (0x82195FF8).
    pub drop: bool,
}

/// 0x824AFFD8: the player's winch for a step. `target` is the object her
/// controls name (+0x19), `cargo` what is on the cable, `far` that the
/// target is more than a metre from the hook.
pub fn player_step(
    w: &Winch,
    pad: &WinchPad,
    target: Option<u32>,
    cargo: Option<u32>,
    far: bool,
) -> PlayerAsk {
    let mut ask = PlayerAsk::default();
    if !pad.on {
        return ask;
    }
    let want = pad.want & 3;
    let state = w.state as u8;
    if want != state {
        // Retracted, asked deployed or deploying; deployed, asked
        // retracted or retracting: the second button.
        ask.press = (w.state == WinchState::Retracted && (want == 1 || want == 2))
            || (w.state == WinchState::Deployed && (want == 0 || want == 3));
    }
    if w.state == WinchState::Deployed
        && pad.hold
        && let Some(t) = target
        && cargo != Some(t)
    {
        ask.hook = true;
        ask.snap = far;
    }
    if !pad.hold && cargo.is_some() {
        ask.drop = true;
    }
    ask
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heli() -> Winch {
        Winch::new(&WinchParams {
            length: 10.0,
            retract_autodrop_when_length_shorter: 2.0,
            ..WinchParams::default()
        })
    }

    #[test]
    fn her_controls_work_the_winch() {
        let mut w = heli();
        let pad = |want, hold| WinchPad {
            on: true,
            want,
            hold,
        };
        // Off: nothing.
        assert_eq!(
            player_step(&w, &WinchPad::default(), Some(5), None, false),
            PlayerAsk::default()
        );
        // Retracted, asked to deploy: the second button.
        assert!(player_step(&w, &pad(2, false), None, None, false).press);
        assert!(!player_step(&w, &pad(3, false), None, None, false).press);
        w.state = WinchState::Deployed;
        assert!(player_step(&w, &pad(0, false), None, None, false).press);
        assert!(!player_step(&w, &pad(2, false), None, None, false).press);
        // Deployed, holding with a target: hook it, snapped if far.
        let a = player_step(&w, &pad(1, true), Some(5), None, true);
        assert!(a.hook && a.snap && !a.drop);
        assert!(!player_step(&w, &pad(1, true), Some(5), Some(5), true).hook);
        // Let go of the button: the cargo drops.
        assert!(player_step(&w, &pad(1, false), Some(5), Some(5), false).drop);
    }

    #[test]
    fn it_is_built_from_its_length() {
        let w = heli();
        assert_eq!((w.max_links, w.autodrop_links), (63, 12));
        assert_eq!(w.state, WinchState::Retracted);
        let d = Winch::new(&WinchParams {
            length: 0.1,
            deployed: true,
            ..WinchParams::default()
        });
        assert_eq!((d.max_links, d.state), (7, WinchState::Deployed));
        assert_eq!(WinchState::named("Deploying"), Some(WinchState::Deploying));
        assert_eq!(WinchState::Retracting.name(), "retracting");
    }

    #[test]
    fn it_pays_out_at_six_and_reels_in_at_three() {
        let mut w = heli();
        w.press_deploy();
        assert_eq!(w.state, WinchState::Deploying);
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        let mut added = 0;
        while w.state == WinchState::Deploying {
            added += w.step(dt, false).add;
            t += dt;
            assert!(t < 5.0);
        }
        // 63 links of 0.16 m at 6 m/s: 1.68 s.
        assert_eq!((w.state, w.links, added), (WinchState::Deployed, 63, 63));
        assert!((t - 63.0 * LINK / DEPLOY_SPEED).abs() < 0.05, "{t}");
        w.press_retract();
        let mut t = 0.0;
        while w.state == WinchState::Retracting {
            w.step(dt, false);
            t += dt;
            assert!(t < 10.0);
        }
        assert_eq!((w.state, w.links), (WinchState::Retracted, 0));
        assert!((t - 63.0 * LINK / RETRACT_SPEED).abs() < 0.1, "{t}");
    }

    #[test]
    fn the_cargo_drops_reeling_in_short_or_stretched() {
        let mut w = heli();
        w.state = WinchState::Deployed;
        w.step(0.1, false);
        w.cargo = true;
        // The second button with more out than the autodrop length reels
        // in; at the autodrop length the cargo drops.
        assert_eq!(w.press_deploy(), Change::default());
        assert_eq!(w.state, WinchState::Retracting);
        let mut dropped_at = None;
        for _ in 0..200 {
            if w.step(1.0 / 60.0, false).drop {
                dropped_at = Some(w.links);
                break;
            }
        }
        assert_eq!(dropped_at, Some(12));
        // Stretched over 0.2 s: the cargo drops, or with none it reels in.
        let mut w = heli();
        w.state = WinchState::Deployed;
        w.step(0.1, false);
        w.cargo = true;
        assert!(!w.step(0.15, true).drop);
        assert!(w.step(0.1, true).drop);
        assert_eq!(w.state, WinchState::Deployed);
        w.step(0.15, true);
        w.step(0.1, true);
        assert_eq!(w.state, WinchState::Retracting);
    }
}
