//! `--flicker-check`: how much of the frame flickers as the camera pans
//! past trees, with the sun's shadow maps on the game's schedule (all four
//! drawn every frame, 360 code 0x821B6A98), on the renderer's earlier
//! staggered one, and with shadows off; and with the game's edge
//! anti-aliasing (PgAntiAliasingFP) on and off.
//!
//! A pixel flickers on a frame when its brightness jumps one way from the
//! frame before and back the other way on the frame after, both by at
//! least [`STEP`] (of 255): a pan moves an edge across a pixel once, so
//! smooth motion rarely does that, while a shadow that comes and goes or a
//! cut-out leaf that blinks does. The rate is such pixels over all pixels
//! and frames. The step and the pans are this check's choices (invented).

use std::path::Path;

use anyhow::Context;
use m2_math::glam::Vec3;
use m2_render::gpu::{ShadowSchedule, WgpuBackend};

use crate::camera::FlyCamera;
use crate::game::Game;

/// A jump in brightness (of 255) that counts towards a flicker.
pub const STEP: i16 = 24;
/// Frames drawn for each variant.
pub const FRAMES: usize = 48;

/// Rec. 601 brightness (0-255) of each pixel of an RGBA frame.
pub fn luma(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .map(|p| {
            let y = 299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2]);
            (y / 1000) as u8
        })
        .collect()
}

/// Flickers counted over a run of frames, and where.
#[derive(Clone, Debug, Default)]
pub struct Flicker {
    /// Per pixel, the frames it flickered on.
    pub counts: Vec<u16>,
    /// Pixel-frames that could flicker (pixels x (frames - 2)).
    pub chances: u64,
}

impl Flicker {
    /// Counts the flickers in `frames` (brightness, all the same size).
    pub fn of(frames: &[Vec<u8>]) -> Self {
        let n = frames.first().map_or(0, Vec::len);
        let mut counts = vec![0u16; n];
        for w in frames.windows(3) {
            for (i, c) in counts.iter_mut().enumerate() {
                let (a, b, d) = (i16::from(w[0][i]), i16::from(w[1][i]), i16::from(w[2][i]));
                let (d1, d2) = (b - a, d - b);
                if d1.abs() >= STEP && d2.abs() >= STEP && (d1 > 0) != (d2 > 0) {
                    *c += 1;
                }
            }
        }
        Self {
            counts,
            chances: n as u64 * frames.len().saturating_sub(2) as u64,
        }
    }

    /// Flickers per pixel-frame.
    pub fn rate(&self) -> f64 {
        let total: u64 = self.counts.iter().map(|&c| u64::from(c)).sum();
        if self.chances == 0 {
            0.0
        } else {
            total as f64 / self.chances as f64
        }
    }

    /// Writes where it flickered as a grey PPM (white: every frame).
    pub fn write_map(
        &self,
        width: u32,
        height: u32,
        frames: usize,
        out: &Path,
    ) -> anyhow::Result<()> {
        let most = frames.saturating_sub(2).max(1) as u32;
        let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
        for &c in &self.counts {
            let v = (u32::from(c) * 255 / most).min(255) as u8;
            ppm.extend([v, v, v]);
        }
        std::fs::write(out, ppm).with_context(|| format!("writing {}", out.display()))
    }
}

/// A camera pan: turn (radians) and sideways step (metres) per frame.
#[derive(Clone, Copy, Debug)]
struct Pan {
    name: &'static str,
    turn: f32,
    step: f32,
}

const STILL: Pan = Pan {
    name: "still",
    turn: 0.0,
    step: 0.0,
};
/// A slow walk past: 3 degrees and 1.2 m a second at 60 frames.
const SLOW: Pan = Pan {
    name: "slow",
    turn: 0.05 * std::f32::consts::PI / 180.0,
    step: 0.02,
};
/// Where to look from and at: from her eye to the crown (4 m up its trunk)
/// of the nearest tree 8-60 m off that nothing static stands in front of,
/// else along her facing.
fn aim(game: &Game) -> Option<(Vec3, Vec3, Option<f32>)> {
    let p = game.scene.player.as_ref()?;
    let eye = p.character.position + Vec3::Y * 1.7;
    let ahead = Vec3::new(p.facing.sin(), 0.0, p.facing.cos());
    let tree = game.scene.world.as_ref().and_then(|w| {
        let mut trees: Vec<(f32, Vec3)> = w
            .index
            .objects
            .iter()
            .filter(|o| m2_engine::world::tree_species(o.entity).is_some())
            .map(|o| o.position + Vec3::Y * CROWN)
            .map(|at| (at.distance(eye), at))
            .filter(|(d, _)| (8.0..=60.0).contains(d))
            .collect();
        trees.sort_by(|a, b| a.0.total_cmp(&b.0));
        trees.into_iter().find(|&(d, at)| {
            // In sight: no wall or ground before the last few metres
            // (its own trunk may stand there).
            w.physics
                .cast_ray(eye, (at - eye) / d, (d - SIGHT_SLACK).max(0.0))
                .is_none()
        })
    });
    Some(match tree {
        Some((d, at)) => (eye, at, Some(d)),
        None => (eye, eye + ahead * 20.0, None),
    })
}

/// Height of a tree's crown above its base, and how far short of it the
/// sight line may be blocked (both this check's choice, invented).
const CROWN: f32 = 4.0;
const SIGHT_SLACK: f32 = 3.0;

/// Runs the check and exits: PASS when a tree is in sight and, on the
/// slow pan at it, the game's picture (every map each frame, anti-aliasing
/// on) flickers less than the same without anti-aliasing, and no more than
/// with the staggered maps.
pub fn check(
    backend: &mut WgpuBackend,
    game: &mut Game,
    fov: f32,
    aspect: f32,
    out: &Path,
) -> anyhow::Result<bool> {
    let Some((eye, at, tree)) = aim(game) else {
        eprintln!("flicker-check: FAIL: no player to look from");
        return Ok(false);
    };
    match tree {
        Some(d) => eprintln!("flicker-check: from {eye:.1} at a tree {d:.1} m off ({at:.1})"),
        None => {
            eprintln!("flicker-check: no tree in sight 8-60 m off; looking ahead from {eye:.1}")
        }
    }
    let (on_foot, camera, shadows, schedule, antialias) = (
        game.on_foot,
        game.camera,
        game.picture.shadows,
        backend.shadow_schedule(),
        backend.antialias(),
    );
    game.on_foot = false;
    let (w, h) = backend.size();
    let stem = out
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("flicker");
    let rate = |backend: &mut WgpuBackend,
                game: &mut Game,
                pan: Pan,
                variant: &str,
                sched: ShadowSchedule,
                shadows: bool,
                antialias: bool|
     -> anyhow::Result<f64> {
        backend.set_shadow_schedule(sched);
        backend.set_antialias(antialias);
        game.picture.shadows = shadows;
        let start = FlyCamera::looking_at(eye, at);
        // Settle: a few frames first so every kept map is this view's.
        game.camera = start;
        for _ in 0..4 {
            crate::render(backend, game, fov, aspect, None)?;
        }
        let mut frames = Vec::with_capacity(FRAMES);
        for k in 0..FRAMES {
            let mut c = start;
            // Back and forth about the tree, so it stays in view.
            let s = (k as f32) - (FRAMES as f32) * 0.5;
            c.yaw += pan.turn * s;
            c.position += start.right() * (pan.step * s);
            game.camera = c;
            crate::render(backend, game, fov, aspect, None)?;
            let pixels = backend.read_pixels()?;
            if k == FRAMES / 2 {
                let shot = out.with_file_name(format!("{stem}-frame-{}-{variant}.ppm", pan.name));
                write_rgb(&pixels, w, h, &shot)?;
            }
            frames.push(luma(&pixels));
        }
        let f = Flicker::of(&frames);
        let r = f.rate();
        let map = out.with_file_name(format!("{stem}-flicker-{}-{variant}.ppm", pan.name));
        f.write_map(w, h, FRAMES, &map)?;
        eprintln!(
            "flicker-check: {} pan, {variant}: {:.5} of pixel-frames flicker (map {})",
            pan.name,
            r,
            map.display()
        );
        Ok(r)
    };
    let every = ShadowSchedule::Every;
    let floor = rate(backend, game, STILL, "game", every, true, true)?;
    let game_rate = rate(backend, game, SLOW, "game", every, true, true)?;
    let no_aa = rate(backend, game, SLOW, "noaa", every, true, false)?;
    let staggered = rate(
        backend,
        game,
        SLOW,
        "staggered",
        ShadowSchedule::Staggered,
        true,
        true,
    )?;
    let off = rate(backend, game, SLOW, "noshadow", every, false, true)?;
    let pass = tree.is_some() && game_rate < no_aa && game_rate <= staggered + 1e-6;
    eprintln!(
        "flicker-check: slow pan: game {game_rate:.5}, no anti-aliasing {no_aa:.5}, staggered {staggered:.5}, shadows off {off:.5}, still {floor:.5}"
    );
    backend.set_shadow_schedule(schedule);
    backend.set_antialias(antialias);
    game.picture.shadows = shadows;
    game.on_foot = on_foot;
    game.camera = camera;
    eprintln!("flicker-check: {}", if pass { "PASS" } else { "FAIL" });
    Ok(pass)
}

/// Writes an RGBA frame as a colour PPM.
fn write_rgb(rgba: &[u8], width: u32, height: u32, out: &Path) -> anyhow::Result<()> {
    let mut ppm = format!("P6\n{width} {height}\n255\n").into_bytes();
    for p in rgba.chunks_exact(4) {
        ppm.extend(&p[..3]);
    }
    std::fs::write(out, ppm).with_context(|| format!("writing {}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pixel_that_blinks_flickers_and_one_that_steps_does_not() {
        // Pixel 0 blinks 0-200-0-200; pixel 1 steps up once and stays;
        // pixel 2 ramps smoothly; pixel 3 wobbles under the step.
        let frames: Vec<Vec<u8>> = vec![
            vec![0, 0, 0, 100],
            vec![200, 0, 60, 110],
            vec![0, 200, 120, 100],
            vec![200, 200, 180, 110],
        ];
        let f = Flicker::of(&frames);
        assert_eq!(f.counts, vec![2, 0, 0, 0]);
        assert_eq!(f.chances, 8);
        assert!((f.rate() - 0.25).abs() < 1e-12);
    }

    #[test]
    fn brightness_weighs_green_most() {
        assert_eq!(
            luma(&[255, 255, 255, 255, 0, 255, 0, 0, 255, 0, 0, 0]),
            vec![255, 149, 76]
        );
    }

    #[test]
    fn too_few_frames_have_no_chance_to_flicker() {
        let f = Flicker::of(&[vec![0, 0], vec![9, 9]]);
        assert_eq!((f.chances, f.rate()), (0, 0.0));
    }
}
