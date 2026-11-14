//! Material animations played on an object (`Object.PlayMaterialAnimation`):
//! keyed values put in place of its materials' own, frame by frame (a
//! burning tree's leaves going black and fading out).
//!
//! **From the 360 code.** Each frame the animation's tracks are sampled
//! and each value written into the instance's own copy of the material's
//! block (0x82425AD0 on the copy 0x82426618 makes), replacing the value
//! there, by the track's parameter hash. The block is the material's
//! `MTRL` constants as the reader lays them out (0x8256DD60): constants
//! 0-2, 3-5, 6-8 and 9-11 as four colours at +0x00, +0x10, +0x20 and
//! +0x30, constant 12 at +0x5C, 13 at +0x58, 14 at +0x50, then the
//! texture transform (constants 20-24 at +0x68 to +0x78, rebuilt by
//! 0x8256EAA8). So, by parameter:
//!
//! - [`DIRECT`] (`0x112A5DA9`, `0x1AA50935`, `0xF02D6A30`): constants
//!   0-2, the surface's direct light colour in our drawing.
//! - [`SPECULAR`] (`0x60945EE0`, `0xDACE9D70`, `0xF618CC0F`): constants
//!   3-5, its highlight colour.
//! - [`OPACITY`] (`0x430556D7`, times 0.01): constant 12, its opacity.
//!
//! The rest the 360 also takes (the ambient and emitted colours, the
//! texture transform, constants 13 and 14) are not drawn from here yet.
//!
//! **Inferred:** values between keys are linear, held after the last
//! ([`ParamTrack::sample`]); a colour is put in place only when all
//! three of its tracks are there (the trees' are).

use m2_assets::material_animation::{MaterialAnimation, MaterialTracks};
use m2_render::SurfaceOverride;

/// The tracks of the direct light colour, red, green, blue.
pub const DIRECT: [u32; 3] = [0x112a_5da9, 0x1aa5_0935, 0xf02d_6a30];
/// The tracks of the highlight colour.
pub const SPECULAR: [u32; 3] = [0x6094_5ee0, 0xdace_9d70, 0xf618_cc0f];
/// The opacity's track, in hundredths.
pub const OPACITY: u32 = 0x4305_56d7;

/// The values of `m`'s tracks at `frame`, in place of its material's.
pub fn surface_at(m: &MaterialTracks, frame: f32) -> SurfaceOverride {
    let track = |p: u32| m.tracks.iter().find(|t| t.param_hash == p);
    let colour = |ps: [u32; 3]| {
        let [r, g, b] = ps.map(track);
        Some([r?.sample(frame), g?.sample(frame), b?.sample(frame)])
    };
    SurfaceOverride {
        direct: colour(DIRECT),
        specular: colour(SPECULAR),
        opacity: track(OPACITY).map(|t| t.sample(frame) * 0.01),
    }
}

/// An animation playing on an object.
#[derive(Debug, Clone)]
pub struct Playing {
    anim: MaterialAnimation,
    looped: bool,
    seconds: f32,
}

impl Playing {
    pub fn new(anim: MaterialAnimation, looped: bool) -> Self {
        Self {
            anim,
            looped,
            seconds: 0.0,
        }
    }

    /// Time passes.
    pub fn advance(&mut self, dt: f32) {
        self.seconds += dt;
    }

    /// The frame it is at: looped round its length, or held at its end.
    pub fn frame(&self) -> f32 {
        let length = self.anim.frames.max(1) as f32;
        let frame = self.seconds * self.anim.fps;
        if self.looped {
            frame % length
        } else {
            frame.min(length)
        }
    }

    /// Its values now, by material name hash.
    pub fn surfaces(&self) -> Vec<(u32, SurfaceOverride)> {
        let frame = self.frame();
        self.anim
            .materials
            .iter()
            .map(|m| (m.material_hash, surface_at(m, frame)))
            .filter(|(_, s)| *s != SurfaceOverride::NONE)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_assets::material_animation::ParamTrack;

    fn track(param_hash: u32, divisor: f32, keys: &[(u16, i16)]) -> ParamTrack {
        ParamTrack {
            param_hash,
            divisor,
            flags: 0,
            keys: keys.to_vec(),
        }
    }

    /// A burning leaf's: its colours and opacity from full to none over
    /// 100 frames.
    fn burning() -> MaterialAnimation {
        let fade = |p| track(p, 32767.0, &[(0, 32767), (100, 0)]);
        let mut tracks: Vec<ParamTrack> =
            DIRECT.iter().chain(&SPECULAR).map(|&p| fade(p)).collect();
        tracks.push(track(OPACITY, 327.67, &[(0, 32767), (100, 0)]));
        MaterialAnimation {
            version: 2,
            frames: 100,
            fps: 10.0,
            materials: vec![MaterialTracks {
                material_hash: 0xabcd,
                tracks,
            }],
        }
    }

    #[test]
    fn the_tracks_replace_the_direct_specular_and_opacity() {
        let mut p = Playing::new(burning(), false);
        let at = |p: &Playing| p.surfaces()[0].1;
        assert_eq!(at(&p).direct, Some([1.0; 3]));
        assert!((at(&p).opacity.unwrap() - 1.0).abs() < 1e-3);
        p.advance(5.0);
        let half = at(&p);
        assert!((half.direct.unwrap()[0] - 0.5).abs() < 1e-3);
        assert!((half.specular.unwrap()[2] - 0.5).abs() < 1e-3);
        assert!((half.opacity.unwrap() - 0.5).abs() < 1e-3);
        // Played once, it holds its end.
        p.advance(60.0);
        assert_eq!(at(&p).opacity, Some(0.0));
        assert_eq!(p.surfaces()[0].0, 0xabcd);
    }

    #[test]
    fn a_looped_one_starts_again_and_a_colour_needs_its_three_tracks() {
        let mut p = Playing::new(burning(), true);
        p.advance(12.5);
        assert!((p.frame() - 25.0).abs() < 1e-3);
        let mut anim = burning();
        anim.materials[0].tracks.remove(0);
        let s = surface_at(&anim.materials[0], 0.0);
        assert_eq!(s.direct, None);
        assert!(s.specular.is_some());
    }
}
