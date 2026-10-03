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
        skin: None,
        instances: None,
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

#[test]
fn skinned_vertices_follow_their_bones() {
    use VertexFormat::*;
    use VertexSemantic::*;
    let Some(mut b) = backend() else { return };
    let layout = VertexLayout::packed(&[
        (Position, Float3),
        (Color, Bgra8),
        (BlendIndices, UInt4x8),
        (BlendWeight, UNorm4x8),
    ])
    .unwrap();
    // A small quad bound wholly to bone 1, coloured red stored B, G, R, A.
    let mut vertices = Vec::new();
    for (x, y) in [
        (-0.25f32, 0.25f32),
        (0.25, 0.25),
        (0.25, -0.25),
        (-0.25, -0.25),
    ] {
        for f in [x, y, 0.0] {
            vertices.extend(f.to_le_bytes());
        }
        vertices.extend([0, 0, 255, 255, 1, 0, 0, 0, 255, 0, 0, 0]);
    }
    let vb = b
        .create_vertex_buffer(
            &VertexBufferDesc {
                layout,
                vertex_count: 4,
                usage: BufferUsage::Static,
            },
            Some(&vertices),
        )
        .unwrap();
    let (_, ib) = quad(&mut b);
    b.set_material(
        MaterialId(1),
        &MaterialDesc {
            lit: false,
            vertex_color: true,
            ..MaterialDesc::default()
        },
    )
    .unwrap();
    let red = [255, 0, 0, 255];
    let sky = [0, 0, 255, 255];

    // Bind pose without a palette.
    b.begin_frame(&view()).unwrap();
    b.draw(&draw(vb, ib, 1, Mat4::IDENTITY)).unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    assert_eq!(pixel(&rgba, 32, 32), red, "bind pose");

    // Bone 1 moves the quad right.
    b.begin_frame(&view()).unwrap();
    let skin = b
        .set_skin(&[
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.6, 0.0, 0.0)),
        ])
        .unwrap();
    b.draw(&DrawCall {
        skin: Some(skin),
        ..draw(vb, ib, 1, Mat4::IDENTITY)
    })
    .unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    assert_eq!(pixel(&rgba, 32, 32), sky, "moved away");
    assert_eq!(pixel(&rgba, 43, 32), red, "moved right");

    // A palette lasts one frame.
    b.begin_frame(&view()).unwrap();
    let stale = b.draw(&DrawCall {
        skin: Some(skin),
        ..draw(vb, ib, 1, Mat4::IDENTITY)
    });
    assert!(matches!(stale, Err(BackendError::StaleSkin { .. })));
    b.end_frame().unwrap();
}

#[test]
fn instanced_draws_repeat_the_geometry() {
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
    // Two small quads, left and right, and nothing in the middle.
    b.begin_frame(&view()).unwrap();
    let small = Mat4::from_scale(Vec3::splat(0.2));
    let instances = b
        .set_skin(&[
            Mat4::from_translation(Vec3::new(-1.0, 0.0, 0.0)) * small,
            Mat4::from_translation(Vec3::new(1.0, 0.0, 0.0)) * small,
        ])
        .unwrap();
    b.draw(&DrawCall {
        instances: Some(instances),
        ..draw(vb, ib, 1, Mat4::IDENTITY)
    })
    .unwrap();
    b.end_frame().unwrap();
    let rgba = b.read_pixels().unwrap();
    let (white, sky) = ([255; 4], [0, 0, 255, 255]);
    assert_eq!(pixel(&rgba, 32, 32), sky, "middle");
    assert_eq!(pixel(&rgba, 14, 32), white, "left");
    assert_eq!(pixel(&rgba, 50, 32), white, "right");
}

#[test]
fn detail_maps_add_grain_up_close() {
    let Some(mut b) = backend() else { return };
    let (vb, ib) = quad(&mut b);
    // Dark and light stripes along x, tiled every metre of world x.
    let texels: Vec<u8> = (0..64)
        .flat_map(|i| {
            if i % 8 < 4 {
                [60, 60, 60, 255]
            } else {
                [200, 200, 200, 255]
            }
        })
        .collect();
    let mips = generate_mips(8, 8, &texels);
    let levels: Vec<&[u8]> = mips.iter().map(Vec::as_slice).collect();
    let detail = b
        .create_texture(
            &TextureDesc {
                width: 8,
                height: 8,
                format: TextureFormat::Rgba8Srgb,
            },
            &levels,
        )
        .unwrap();
    let plain = MaterialDesc {
        lit: false,
        color: [0.5, 0.5, 0.5, 1.0],
        ..MaterialDesc::default()
    };
    b.set_material(MaterialId(1), &plain).unwrap();
    b.set_material(
        MaterialId(2),
        &MaterialDesc {
            detail: Some(DetailMap {
                texture: detail,
                tile: 1.0,
            }),
            ..plain
        },
    )
    .unwrap();
    let row = |b: &mut WgpuBackend, material| {
        b.begin_frame(&view()).unwrap();
        b.draw(&draw(vb, ib, material, Mat4::IDENTITY)).unwrap();
        b.end_frame().unwrap();
        let rgba = b.read_pixels().unwrap();
        let reds: Vec<u8> = (18..46).map(|x| pixel(&rgba, x, 32)[0]).collect();
        (*reds.iter().min().unwrap(), *reds.iter().max().unwrap())
    };
    let (lo, hi) = row(&mut b, 1);
    assert!(hi - lo <= 1, "flat without detail: {lo}..{hi}");
    let (lo, hi) = row(&mut b, 2);
    assert!(hi - lo > 40, "striped with detail: {lo}..{hi}");
}

#[test]
fn overlays_draw_over_the_finished_world() {
    let Some(mut backend) = backend() else { return };
    backend.begin_frame(&view()).unwrap();
    let mut called = 0;
    backend
        .end_frame_with(&mut |t| {
            called += 1;
            assert_eq!(t.size, (SIZE, SIZE));
            // A pass that keeps what the world drew.
            let mut pass =
                t.encoder
                    .begin_render_pass(&m2_render::gpu::wgpu::RenderPassDescriptor {
                        label: Some("overlay"),
                        color_attachments: &[Some(
                            m2_render::gpu::wgpu::RenderPassColorAttachment {
                                view: t.view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: m2_render::gpu::wgpu::Operations {
                                    load: m2_render::gpu::wgpu::LoadOp::Load,
                                    store: m2_render::gpu::wgpu::StoreOp::Store,
                                },
                            },
                        )],
                        ..Default::default()
                    });
            pass.set_scissor_rect(0, 0, SIZE / 2, SIZE);
        })
        .unwrap();
    assert_eq!(called, 1);
    // The world (all sky) is still there.
    let pixels = backend.read_pixels().unwrap();
    let sky = &pixels[..4];
    assert!(sky[2] > sky[0], "{sky:?}");
}
