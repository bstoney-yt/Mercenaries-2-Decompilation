use super::*;

fn gradient(kind: GradientKind, linear_rgb: bool, stops: &[(u8, [u8; 4])]) -> Gradient {
    Gradient {
        kind,
        matrix: Matrix::IDENTITY,
        linear_rgb,
        stops: stops.to_vec(),
        index: 0,
        image: None,
    }
}

const BLACK: [u8; 4] = [0, 0, 0, 255];
const WHITE: [u8; 4] = [255, 255, 255, 255];

#[test]
fn a_ramp_reaches_each_stop_one_entry_early() {
    // Entry r0 + j - 1 holds the mix j / n of the way: entry 0 is already
    // 1/255 along, entry 254 is the end colour.
    let g = gradient(GradientKind::Linear, false, &[(0, BLACK), (255, WHITE)]);
    let r = g.ramp();
    assert_eq!(r[0], [1, 1, 1, 255]);
    assert_eq!(r[127], [128, 128, 128, 255]);
    assert_eq!(r[254], WHITE);
    assert_eq!(r[255], WHITE);
}

#[test]
fn a_ramp_holds_its_ends_and_rounds_towards_the_earlier_colour() {
    let red = [255, 0, 0, 255];
    let clear_blue = [0, 0, 255, 0];
    let g = gradient(GradientKind::Linear, false, &[(64, red), (192, clear_blue)]);
    let r = g.ramp();
    assert!(r[..64].iter().all(|c| *c == red));
    // 255 - 255 / 128 in whole numbers: 254; blue 255 / 128: 1.
    assert_eq!(r[64], [254, 0, 1, 254]);
    assert_eq!(r[191], clear_blue);
    assert!(r[192..].iter().all(|c| *c == clear_blue));
}

#[test]
fn a_stop_behind_the_one_before_counts_from_it() {
    let red = [255, 0, 0, 255];
    let green = [0, 255, 0, 255];
    let g = gradient(
        GradientKind::Linear,
        false,
        &[(100, red), (50, green), (200, BLACK)],
    );
    let r = g.ramp();
    // Nothing between red and green; green to black from 100.
    assert_eq!(r[99], red);
    assert_eq!(r[100], [0, 255 - 255 / 100, 0, 255]);
    assert_eq!(r[199], BLACK);
}

#[test]
fn one_stop_fills_the_ramp() {
    let g = gradient(GradientKind::Radial, false, &[(128, [9, 8, 7, 6])]);
    assert!(g.ramp().iter().all(|c| *c == [9, 8, 7, 6]));
}

#[test]
fn linear_rgb_mixes_brighter_in_the_middle() {
    let g = gradient(GradientKind::Linear, true, &[(0, BLACK), (255, WHITE)]);
    let r = g.ramp();
    // 65535 * 128 / 255 back through the power 1 / 2.17.
    let want = ((32896.0f32 / 65535.0).powf(1.0 / 2.17) * 255.0 + 0.5).floor() as u8;
    assert_eq!(want, 186);
    assert_eq!(r[127], [want, want, want, 255]);
    assert_eq!(r[254], WHITE);
    // Alpha mixes as is.
    let g = gradient(
        GradientKind::Linear,
        true,
        &[(0, [0, 0, 0, 0]), (255, [0, 0, 0, 255])],
    );
    assert_eq!(g.ramp()[127][3], 128);
}

#[test]
fn radial_textures_grow_with_the_steepest_change() {
    let soft = gradient(GradientKind::Radial, false, &[(0, BLACK), (255, WHITE)]);
    assert_eq!(soft.radial_size(), 32);
    let hard = gradient(GradientKind::Radial, false, &[(100, BLACK), (101, WHITE)]);
    assert_eq!(hard.radial_size(), 256);
    let flat = gradient(GradientKind::Radial, false, &[(0, WHITE), (255, WHITE)]);
    assert_eq!(flat.radial_size(), 64);
    // 3 per ratio: sqrt(3.18 * 5) = 3.99, entry 3; linear RGB counts it
    // as 4.5: sqrt(4.68 * 5) = 4.84, entry 4.
    let red = [255, 0, 0, 255];
    let steep = gradient(GradientKind::Radial, false, &[(0, BLACK), (85, red)]);
    assert_eq!(steep.radial_size(), 32);
    let lin = gradient(GradientKind::Radial, true, &[(0, BLACK), (85, red)]);
    assert_eq!(lin.radial_size(), 64);
    // A focal point near the edge: 1 / (1.01 - 0.95) per ratio.
    let far = gradient(
        GradientKind::Focal(0.95),
        false,
        &[(0, BLACK), (255, WHITE)],
    );
    assert_eq!(far.radial_size(), 128);
}

#[test]
fn a_radial_texture_has_the_last_colour_round_its_edge() {
    let g = gradient(
        GradientKind::Radial,
        false,
        &[(0, [255, 0, 0, 255]), (255, [0, 0, 255, 255])],
    );
    let (w, h, px) = g.pixels();
    assert_eq!((w, h), (32, 32));
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [px[i], px[i + 1], px[i + 2], px[i + 3]]
    };
    let ramp = g.ramp();
    for i in 0..32 {
        for p in [at(i, 0), at(31, i), at(i, 31), at(0, i)] {
            assert_eq!(p, ramp[255]);
        }
    }
    // Texel 15 has its centre half a texel from the middle (16) both ways:
    // sqrt(0.5) / 15 of the way out, entry 12.
    assert_eq!(at(15, 15), ramp[12]);
    // 14.509 / 15 out: entry 248.
    assert_eq!(at(1, 16), ramp[248]);
}

#[test]
fn a_focal_gradient_at_the_middle_is_a_radial_one() {
    let stops = [(0, BLACK), (255, WHITE)];
    let radial = gradient(GradientKind::Radial, false, &stops);
    let focal = gradient(GradientKind::Focal(0.0), false, &stops);
    assert_eq!(radial.pixels(), focal.pixels());
    // Off centre, the colours start at the focal point: middle 16, radius
    // 15, focal point at 23.5 (the centre of texel 23).
    let off = gradient(GradientKind::Focal(0.5), false, &stops);
    let (w, _, px) = off.pixels();
    let at = |x: u32, y: u32| px[((y * w + x) * 4) as usize];
    assert!(at(23, 16) < 20);
    assert!(at(8, 16) > at(16, 16));
}

#[test]
fn the_square_maps_onto_the_texture() {
    let linear = gradient(GradientKind::Linear, false, &[(0, BLACK), (255, WHITE)]);
    let inv = Matrix::IDENTITY;
    assert_eq!(linear.uv(&inv, [-1.0, 0.0]), [0.0, 0.5]);
    assert_eq!(linear.uv(&inv, [0.0, 0.3]), [0.5, 0.5]);
    assert_eq!(linear.uv(&inv, [1.0, 0.0]), [1.0, 0.5]);
    // A 32-texel radial texture: the square edge falls on its border,
    // 16384 / (32768 / 30 = 1092) texels from the middle.
    let radial = gradient(GradientKind::Radial, false, &[(0, BLACK), (255, WHITE)]);
    let [u, v] = radial.uv(&inv, [1.0, 0.0]);
    assert!((u - (16384.0 / 1092.0 + 16.0) / 32.0).abs() < 1e-6);
    assert_eq!(v, 0.5);
    // An exported texture keeps its own size.
    let mut exported = radial.clone();
    exported.image = Some(GradientImage {
        texture: "m_G0".into(),
        size: 64,
    });
    let [u, _] = exported.uv(&inv, [-1.0, 0.0]);
    assert!((u - (-16384.0 / 528.0 + 32.0) / 64.0).abs() < 1e-6);
    assert_eq!(exported.texture_name(), "m_G0");
}

#[test]
fn a_made_texture_is_named_by_its_gradient() {
    let g = gradient(
        GradientKind::Focal(-0.25),
        true,
        &[(3, [1, 2, 3, 4]), (250, [250, 251, 252, 253])],
    );
    let name = g.texture_name();
    assert!(name.starts_with(MADE_PREFIX));
    assert_eq!(Gradient::from_texture_name(&name), Some(g.clone()));
    assert_eq!(made_pixels(&name), Some(g.pixels()));
    assert_eq!(made_pixels("shell_G0"), None);
}

#[test]
fn a_morph_mixes_its_two_gradients() {
    let a = gradient(GradientKind::Focal(0.0), false, &[(0, BLACK), (100, WHITE)]);
    let b = gradient(
        GradientKind::Focal(1.0),
        false,
        &[(10, WHITE), (201, BLACK)],
    );
    let m = a.mix(&b, 0.5);
    assert_eq!(
        m.stops,
        vec![(5, [128, 128, 128, 255]), (151, [128, 128, 128, 255])]
    );
    assert_eq!(m.kind, GradientKind::Focal(0.5));
    assert_eq!(m.image, None);
}
