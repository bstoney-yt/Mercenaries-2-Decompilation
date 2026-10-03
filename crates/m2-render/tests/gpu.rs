//! Frames through the wgpu backend, rendered offscreen and read back.
//!
//! Needs a GPU or a software rasteriser (lavapipe, WARP); without one the
//! tests print why and pass, so CI machines without graphics still run the
//! rest of the suite.

#![cfg(feature = "wgpu")]

use m2_math::glam::{Mat4, Vec3};
use m2_render::gpu::WgpuBackend;
use m2_render::*;

const SIZE: u32 = 64;

fn backend() -> Option<WgpuBackend> {
    match WgpuBackend::headless(SIZE, SIZE) {
        Ok(b) => {
            eprintln!("adapter: {:?}", b.adapter_summary());
            Some(b)
        }
        Err(e) => {
            eprintln!("skipping: no GPU adapter ({e})");
            None
        }
    }
}

/// A unit quad in the xy plane facing -z (towards a camera at -z looking
/// along +z), wound clockwise as seen from the camera.
fn quad(backend: &mut WgpuBackend) -> (VertexBufferHandle, IndexBufferHandle) {
    use VertexFormat::*;
    use VertexSemantic::*;
    let layout =
        VertexLayout::packed(&[(Position, Float3), (Normal, Float3), (TexCoord(0), Float2)])
            .unwrap();
    #[rustfmt::skip]
    let vertices: [f32; 32] = [
        -1.0,  1.0, 0.0,  0.0, 0.0, -1.0,  0.0, 0.0,
         1.0,  1.0, 0.0,  0.0, 0.0, -1.0,  1.0, 0.0,
         1.0, -1.0, 0.0,  0.0, 0.0, -1.0,  1.0, 1.0,
        -1.0, -1.0, 0.0,  0.0, 0.0, -1.0,  0.0, 1.0,
    ];
    let vb = backend
        .create_vertex_buffer(
            &VertexBufferDesc {
                layout,
                vertex_count: 4,
                usage: BufferUsage::Static,
            },
            Some(bytemuck_bytes(&vertices)),
        )
        .unwrap();
    let indices: Vec<u8> = [0u16, 1, 2, 0, 2, 3]
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let ib = backend
        .create_index_buffer(
            &IndexBufferDesc {
                format: IndexFormat::U16,
                index_count: 6,
                usage: BufferUsage::Static,
            },
            Some(&indices),
        )
        .unwrap();
    (vb, ib)
}

fn bytemuck_bytes(f: &[f32]) -> &[u8] {
    // SAFETY: f32 has no padding or invalid bit patterns as bytes.
    unsafe { std::slice::from_raw_parts(f.as_ptr().cast(), std::mem::size_of_val(f)) }
}

fn view() -> View {
    let mut v = View::perspective(
        Vec3::new(0.0, 0.0, -3.0),
        Vec3::Z,
        60f32.to_radians(),
        1.0,
        0.1,
        100.0,
    );
    v.lighting.sky_color = Vec3::new(0.0, 0.0, 1.0);
    v
}

fn pixel(rgba: &[u8], x: u32, y: u32) -> [u8; 4] {
    let i = ((y * SIZE + x) * 4) as usize;
    rgba[i..i + 4].try_into().unwrap()
}

fn draw(vb: VertexBufferHandle, ib: IndexBufferHandle, material: u32, world: Mat4) -> DrawCall {
    DrawCall {
        material: MaterialId(material),
        topology: Topology::TriangleList,
        vertex_buffer: vb,
        geometry: Geometry::Indexed {
            buffer: ib,
            first: 0,
            count: 6,
            base_vertex: 0,
        },
        world,
    }
}

#[test]
fn clears_to_the_sky_colour() {
    let Some(mut b) = backend() else { return };
    b.begin_frame(&view()).unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    assert_eq!(pixel(&rgba, 5, 5), [0, 0, 255, 255]);
}

#[test]
fn draws_a_textured_unlit_quad() {
    let Some(mut b) = backend() else { return };
    let (vb, ib) = quad(&mut b);
    // 8x8 in quadrants: red, green / white, black. Quadrants rather than
    // single texels so bilinear filtering doesn't blend the probes.
    let quadrant = |x: u32, y: u32| match (x < 4, y < 4) {
        (true, true) => [255, 0, 0, 255],
        (false, true) => [0, 255, 0, 255],
        (true, false) => [255, 255, 255, 255],
        (false, false) => [0, 0, 0, 255],
    };
    let texels: Vec<u8> = (0..64).flat_map(|i| quadrant(i % 8, i / 8)).collect();
    let tex = b
        .create_texture(
            &TextureDesc {
                width: 8,
                height: 8,
                format: TextureFormat::Rgba8Srgb,
            },
            &[&texels],
        )
        .unwrap();
    b.set_material(
        MaterialId(1),
        &MaterialDesc {
            texture: Some(tex),
            lit: false,
            ..MaterialDesc::default()
        },
    )
    .unwrap();
    b.begin_frame(&view()).unwrap();
    b.draw(&draw(vb, ib, 1, Mat4::IDENTITY)).unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    // Texture v runs down the screen, as in Direct3D.
    assert_eq!(pixel(&rgba, 24, 24), [255, 0, 0, 255], "top left");
    assert_eq!(pixel(&rgba, 40, 24), [0, 255, 0, 255], "top right");
    assert_eq!(pixel(&rgba, 24, 40), [255, 255, 255, 255], "bottom left");
    assert_eq!(pixel(&rgba, 40, 40), [0, 0, 0, 255], "bottom right");
    assert_eq!(pixel(&rgba, 2, 2), [0, 0, 255, 255], "sky outside");
    assert_eq!(b.live_buffers().texture, 1);
}

#[test]
fn culls_counter_clockwise_faces_unless_double_sided() {
    let Some(mut b) = backend() else { return };
    let (vb, ib) = quad(&mut b);
    let white = MaterialDesc {
        lit: false,
        ..MaterialDesc::default()
    };
    b.set_material(MaterialId(1), &white).unwrap();
    b.set_material(
        MaterialId(2),
        &MaterialDesc {
            double_sided: true,
            ..white
        },
    )
    .unwrap();
    // Turned around, the quad shows its back.
    let turned = Mat4::from_rotation_y(std::f32::consts::PI);
    for (material, visible) in [(1, false), (2, true)] {
        b.begin_frame(&view()).unwrap();
        b.draw(&draw(vb, ib, material, turned)).unwrap();
        b.end_frame().unwrap();
        let centre = pixel(&b.read_pixels().unwrap(), 32, 32);
        let expected = if visible { [255; 4] } else { [0, 0, 255, 255] };
        assert_eq!(centre, expected, "material {material}");
    }
}

#[test]
fn nearer_draws_hide_farther_ones() {
    let Some(mut b) = backend() else { return };
    let (vb, ib) = quad(&mut b);
    let colour = |c: [f32; 4]| MaterialDesc {
        color: c,
        lit: false,
        ..MaterialDesc::default()
    };
    b.set_material(MaterialId(1), &colour([1.0, 0.0, 0.0, 1.0]))
        .unwrap();
    b.set_material(MaterialId(2), &colour([0.0, 1.0, 0.0, 1.0]))
        .unwrap();
    // Near red first, then far green: depth testing must keep red.
    b.begin_frame(&view()).unwrap();
    b.draw(&draw(vb, ib, 1, Mat4::IDENTITY)).unwrap();
    b.draw(&draw(
        vb,
        ib,
        2,
        Mat4::from_translation(Vec3::new(0.0, 0.0, 50.0)) * Mat4::from_scale(Vec3::splat(40.0)),
    ))
    .unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    assert_eq!(pixel(&rgba, 32, 32), [255, 0, 0, 255]);
    assert_eq!(pixel(&rgba, 2, 2), [0, 255, 0, 255]);
}

#[test]
fn many_draws_grow_the_uniform_buffer() {
    let Some(mut b) = backend() else { return };
    let (vb, ib) = quad(&mut b);
    b.begin_frame(&view()).unwrap();
    for i in 0..3000 {
        let x = (i % 60) as f32 * 0.1 - 3.0;
        b.draw(&draw(
            vb,
            ib,
            0,
            Mat4::from_translation(Vec3::new(x, 0.0, 10.0)) * Mat4::from_scale(Vec3::splat(0.05)),
        ))
        .unwrap();
    }
    b.end_frame().unwrap();
    b.read_pixels().unwrap();
    assert_eq!(b.frames(), 1);
}

#[test]
fn odd_index_writes_keep_their_neighbours() {
    let Some(mut b) = backend() else { return };
    let (vb, ib) = quad(&mut b);
    b.set_material(
        MaterialId(1),
        &MaterialDesc {
            lit: false,
            ..MaterialDesc::default()
        },
    )
    .unwrap();
    // Rewrite index 3 alone (bytes 6..8, not 4-aligned) with its own value.
    b.write_index_buffer(ib, 6, &0u16.to_le_bytes()).unwrap();
    b.begin_frame(&view()).unwrap();
    b.draw(&draw(vb, ib, 1, Mat4::IDENTITY)).unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    assert_eq!(pixel(&rgba, 24, 40), [255; 4]);
    assert_eq!(pixel(&rgba, 40, 24), [255; 4]);
}
