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
//! 0x8256EAA8). So, by parameter ([`PARAMS`], each the `MTRL` constant it
//! stands for):
//!
//! - constants 0-2 (`0x112A5DA9`, `0x1AA50935`, `0xF02D6A30`): the
//!   surface's direct light colour in our drawing; 3-5 (`0x60945EE0`,
//!   `0xDACE9D70`, `0xF618CC0F`) its highlight; 6-8 (`0x2B7E4C33`,
//!   `0x5B716F07`, `0xA99CC3DE`) its ambient light colour; 9-11
//!   (`0x32FA4F7A`, `0x59372A4A`, `0xE4F68E1D`) the colour it gives off.
//! - constant 12 (`0x430556D7`, times 0.01): its opacity; 13 (`0xADBF6004`,
//!   at least 0.0001), 14 (`0x63AE7E10`, times 0.01) and 15
//!   (`0xA7B8281C`, times 0.001) are kept but not drawn from here.
//! - constants 20-24, the texture transform: offset u (`0x099A56FA`) and
//!   v (`0x62F3FCEB`), scale u (`0xD5D0A7E8`) and v (`0x7F8B8755`), and
//!   rotation (`0xE7810CC5`, in degrees: times 0.0174533). 0x8256EAA8
//!   makes two rows of them ([`uv_rows`]): the coordinates less the offset
//!   and a half, scaled, turned by the rotation (its 8192-step sine table
//!   at 0x82B56558), and a half added back.
//!
//! A value an animation leaves alone is the material's own: the copy
//! starts as the material ([`surface`]).
//!
//! **Inferred:** values between keys are linear, held after the last
//! ([`ParamTrack::sample`](m2_assets::material_animation::ParamTrack::sample));
//! that the shaders take the rows to move the texture coordinates (the
//! draw setup that hands them over is not traced; constants 20-24
//! default to no move: 0, 0, 1, 1, 0).

use m2_assets::material_animation::{MaterialAnimation, MaterialTracks};
use m2_assets::model::MATERIAL_CONSTANTS;
use m2_render::SurfaceOverride;

/// What an animation puts in place of each of a material's constants.
pub type Constants = [Option<f32>; MATERIAL_CONSTANTS];

/// Each parameter the 360 takes (0x82425AD0): its hash, the constant it
/// stands for and the scale on its value.
pub const PARAMS: [(u32, usize, f32); 21] = [
    (0x112a_5da9, 0, 1.0),
    (0x1aa5_0935, 1, 1.0),
    (0xf02d_6a30, 2, 1.0),
    (0x6094_5ee0, 3, 1.0),
    (0xdace_9d70, 4, 1.0),
    (0xf618_cc0f, 5, 1.0),
    (0x2b7e_4c33, 6, 1.0),
    (0x5b71_6f07, 7, 1.0),
    (0xa99c_c3de, 8, 1.0),
    (0x32fa_4f7a, 9, 1.0),
    (0x5937_2a4a, 10, 1.0),
    (0xe4f6_8e1d, 11, 1.0),
    (0x4305_56d7, 12, 0.01),
    (0xadbf_6004, 13, 1.0),
    (0x63ae_7e10, 14, 0.01),
    (0xa7b8_281c, 15, 0.001),
    (0x099a_56fa, 20, 1.0),
    (0x62f3_fceb, 21, 1.0),
    (0xd5d0_a7e8, 22, 1.0),
    (0x7f8b_8755, 23, 1.0),
    (0xe781_0cc5, 24, 0.017_453_292),
];

/// The values of `m`'s tracks at `frame`, by constant.
pub fn constants_at(m: &MaterialTracks, frame: f32) -> Constants {
    let mut out: Constants = [None; MATERIAL_CONSTANTS];
    for t in &m.tracks {
        if let Some(&(_, k, scale)) = PARAMS.iter().find(|p| p.0 == t.param_hash) {
            let v = t.sample(frame) * scale;
            out[k] = Some(if k == 13 { v.max(0.0001) } else { v });
        }
    }
    out
}

/// The texture transform's rows for offset `(ou, ov)`, scale `(su, sv)`
/// and rotation `turn` (radians), as 0x8256EAA8 makes them:
/// `u' = su cos(t) (u - u0) + sv sin(t) (v - v0) + 0.5` and
/// `v' = -su sin(t) (u - u0) + sv cos(t) (v - v0) + 0.5`, with
/// `u0 = ou + 0.5`, `v0 = ov + 0.5`.
pub fn uv_rows(ou: f32, ov: f32, su: f32, sv: f32, turn: f32) -> [[f32; 3]; 2] {
    let (s, c) = turn.sin_cos();
    let (u0, v0) = (ou + 0.5, ov + 0.5);
    let (a, b) = (su * c, sv * s);
    let (d, e) = (-su * s, sv * c);
    [
        [a, b, 0.5 - (a * u0 + b * v0)],
        [d, e, 0.5 - (d * u0 + e * v0)],
    ]
}

/// The draw's values for a material of constants `base` with `over` put
/// in place: each colour, the opacity and the texture transform an
/// animation touches, the rest of each taken from the material.
pub fn surface(base: &[f32; MATERIAL_CONSTANTS], over: &Constants) -> SurfaceOverride {
    let value = |k: usize| over[k].unwrap_or(base[k]);
    let colour = |k: usize| {
        over[k..k + 3]
            .iter()
            .any(Option::is_some)
            .then(|| [value(k), value(k + 1), value(k + 2)])
    };
    SurfaceOverride {
        direct: colour(0),
        specular: colour(3),
        ambient: colour(6),
        emissive: colour(9),
        opacity: over[12].map(|o| o.clamp(0.0, 1.0)),
        uv: over[20..25]
            .iter()
            .any(Option::is_some)
            .then(|| uv_rows(value(20), value(21), value(22), value(23), value(24))),
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
    pub fn surfaces(&self) -> Vec<(u32, Constants)> {
        let frame = self.frame();
        self.anim
            .materials
            .iter()
            .map(|m| (m.material_hash, constants_at(m, frame)))
            .filter(|(_, c)| c.iter().any(Option::is_some))
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

    /// A burning leaf's: its direct and highlight colours and opacity from
    /// full to none over 100 frames.
    fn burning() -> MaterialAnimation {
        let fade = |p| track(p, 32767.0, &[(0, 32767), (100, 0)]);
        let mut tracks: Vec<ParamTrack> = PARAMS[..6].iter().map(|p| fade(p.0)).collect();
        tracks.push(track(0x4305_56d7, 327.67, &[(0, 32767), (100, 0)]));
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
    fn the_tracks_replace_their_constants() {
        let mut p = Playing::new(burning(), false);
        let base = [0.5; MATERIAL_CONSTANTS];
        let at = |p: &Playing| surface(&base, &p.surfaces()[0].1);
        assert_eq!(at(&p).direct, Some([1.0; 3]));
        assert!((at(&p).opacity.unwrap() - 1.0).abs() < 1e-3);
        p.advance(5.0);
        let half = at(&p);
        assert!((half.direct.unwrap()[0] - 0.5).abs() < 1e-3);
        assert!((half.specular.unwrap()[2] - 0.5).abs() < 1e-3);
        assert!((half.opacity.unwrap() - 0.5).abs() < 1e-3);
        // Untouched: the material's own (none put in place).
        assert_eq!((half.ambient, half.emissive, half.uv), (None, None, None));
        // Played once, it holds its end.
        p.advance(60.0);
        assert_eq!(at(&p).opacity, Some(0.0));
        assert_eq!(p.surfaces()[0].0, 0xabcd);
    }

    #[test]
    fn a_colour_partly_animated_keeps_the_materials_rest() {
        let mut anim = burning();
        // Only the emitted colour's red, at 2.
        anim.materials[0].tracks = vec![track(0x32fa_4f7a, 1.0, &[(0, 2)])];
        let mut base = [0.0; MATERIAL_CONSTANTS];
        base[10] = 0.25;
        base[11] = 0.75;
        let s = surface(&base, &constants_at(&anim.materials[0], 0.0));
        assert_eq!(s.emissive, Some([2.0, 0.25, 0.75]));
        assert_eq!((s.direct, s.ambient), (None, None));
    }

    /// 0x8256EAA8's rows: none moves nothing; an offset moves the
    /// texture; a scale and a turn go about the middle.
    #[test]
    fn the_texture_transform_is_the_360s() {
        let apply = |r: [[f32; 3]; 2], u: f32, v: f32| {
            (
                r[0][0] * u + r[0][1] * v + r[0][2],
                r[1][0] * u + r[1][1] * v + r[1][2],
            )
        };
        let close =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
        let none = uv_rows(0.0, 0.0, 1.0, 1.0, 0.0);
        assert!(close(apply(none, 0.3, 0.7), (0.3, 0.7)));
        let moved = uv_rows(0.25, 0.0, 1.0, 1.0, 0.0);
        assert!(close(apply(moved, 0.3, 0.7), (0.05, 0.7)));
        let doubled = uv_rows(0.0, 0.0, 2.0, 2.0, 0.0);
        assert!(close(apply(doubled, 0.5, 0.5), (0.5, 0.5)));
        assert!(close(apply(doubled, 1.0, 0.5), (1.5, 0.5)));
        let quarter = uv_rows(0.0, 0.0, 1.0, 1.0, std::f32::consts::FRAC_PI_2);
        assert!(close(apply(quarter, 1.0, 0.5), (0.5, 0.0)));
        // The rotation track is in degrees.
        let mut anim = burning();
        anim.materials[0].tracks = vec![track(0xe781_0cc5, 1.0, &[(0, 90)])];
        let c = constants_at(&anim.materials[0], 0.0);
        assert!((c[24].unwrap() - std::f32::consts::FRAC_PI_2).abs() < 1e-4);
    }
}
