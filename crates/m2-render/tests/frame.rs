//! End-to-end frames through the draw queue and the null backends.

use m2_math::glam::{Mat4, Vec3};
use m2_render::*;

fn triangle_buffers(backend: &mut dyn RenderBackend) -> (VertexBufferHandle, IndexBufferHandle) {
    let layout = VertexLayout::packed(&[(VertexSemantic::Position, VertexFormat::Float3)]).unwrap();
    let positions: [f32; 9] = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let bytes: Vec<u8> = positions.iter().flat_map(|f| f.to_le_bytes()).collect();
    let vb = backend
        .create_vertex_buffer(
            &VertexBufferDesc {
                layout,
                vertex_count: 3,
                usage: BufferUsage::Static,
            },
            Some(&bytes),
        )
        .unwrap();
    let indices: Vec<u8> = [0u16, 1, 2].iter().flat_map(|i| i.to_le_bytes()).collect();
    let ib = backend
        .create_index_buffer(
            &IndexBufferDesc {
                format: IndexFormat::U16,
                index_count: 3,
                usage: BufferUsage::Static,
            },
            Some(&indices),
        )
        .unwrap();
    (vb, ib)
}

fn call(vb: VertexBufferHandle, ib: IndexBufferHandle, material: u32) -> DrawCall {
    DrawCall {
        material: MaterialId(material),
        topology: Topology::TriangleList,
        vertex_buffer: vb,
        geometry: Geometry::Indexed {
            buffer: ib,
            first: 0,
            count: 3,
            base_vertex: 0,
        },
        world: Mat4::IDENTITY,
    }
}

fn prim(call: DrawCall, layer: Layer, translucent: bool, z: f32) -> Primitive {
    Primitive {
        call,
        layer,
        translucent,
        sort_origin: Vec3::new(0.0, 0.0, z),
        user: 0,
    }
}

#[test]
fn frame_draws_in_key_order() {
    let mut backend = RecordingBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);
    let view = View {
        near: 1.0,
        far: 101.0,
        ..View::default()
    };

    let mut queue = DrawQueue::new();
    // Pushed deliberately out of order; materials label the expected slot.
    queue.push(prim(call(vb, ib, 6), Layer::OVERLAY, false, 5.0));
    queue.push(prim(call(vb, ib, 4), Layer::WORLD, true, 10.0)); // far glass
    queue.push(prim(call(vb, ib, 3), Layer::WORLD, false, 90.0)); // material 3
    queue.push(prim(call(vb, ib, 2), Layer::WORLD, false, 80.0)); // material 2, far
    queue.push(prim(call(vb, ib, 5), Layer::WORLD, true, 5.0)); // near glass
    queue.push(prim(call(vb, ib, 2), Layer::WORLD, false, 20.0)); // material 2, near
    queue.push(prim(call(vb, ib, 1), Layer::BACKGROUND, false, 100.0));

    backend.begin_frame(&view).unwrap();
    let stats = queue.flush(&view, &mut backend).unwrap();
    backend.end_frame().unwrap();

    let order: Vec<u32> = backend.draws().map(|d| d.material.0).collect();
    assert_eq!(order, [1, 2, 2, 3, 4, 5, 6]);
    assert_eq!(
        stats,
        SubmitStats {
            draws: 7,
            material_changes: 6
        }
    );
    assert!(queue.is_empty());
    assert_eq!(backend.inner().frames(), 1);
    assert!(matches!(backend.commands()[2], Command::BeginFrame(_)));
    assert_eq!(backend.commands().last(), Some(&Command::EndFrame));
}

#[test]
fn queue_steps_must_run_in_order() {
    let mut backend = NullBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);
    let mut queue = DrawQueue::new();
    queue.push(prim(call(vb, ib, 1), Layer::WORLD, false, 1.0));

    assert_eq!(queue.sort(), Err(QueueError::NotKeyed));
    queue.assign_keys(&View::default());
    assert_eq!(queue.submit(&mut backend), Err(QueueError::NotSorted));
    queue.sort().unwrap();

    // A push after sorting invalidates the keys.
    queue.push(prim(call(vb, ib, 2), Layer::WORLD, false, 1.0));
    assert_eq!(queue.submit(&mut backend), Err(QueueError::NotSorted));
}

#[test]
fn draw_outside_frame_is_rejected_and_queue_still_clears() {
    let mut backend = RecordingBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);
    let mut queue = DrawQueue::new();
    queue.push(prim(call(vb, ib, 1), Layer::WORLD, false, 1.0));

    let err = queue.flush(&View::default(), &mut backend).unwrap_err();
    assert_eq!(err, QueueError::Backend(BackendError::NotInFrame("draw")));
    assert!(queue.is_empty());
    assert_eq!(backend.draws().count(), 0);
}

#[test]
fn frames_do_not_nest() {
    let mut backend = NullBackend::new();
    backend.begin_frame(&View::default()).unwrap();
    assert_eq!(
        backend.begin_frame(&View::default()),
        Err(BackendError::FrameAlreadyOpen)
    );
    backend.end_frame().unwrap();
    assert_eq!(
        backend.end_frame(),
        Err(BackendError::NotInFrame("end_frame"))
    );
}

#[test]
fn stale_handles_are_rejected_after_slot_reuse() {
    let mut backend = NullBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);
    backend.destroy_vertex_buffer(vb).unwrap();
    assert!(backend.destroy_vertex_buffer(vb).is_err());

    // The new buffer reuses the slot but not the generation.
    let (vb2, _) = triangle_buffers(&mut backend);
    assert_eq!(vb2.0.index, vb.0.index);
    assert_ne!(vb2, vb);
    assert!(matches!(
        backend.write_vertex_buffer(vb, 0, &[0; 4]),
        Err(BackendError::InvalidHandle(_))
    ));

    backend.begin_frame(&View::default()).unwrap();
    assert!(matches!(
        backend.draw(&call(vb, ib, 1)),
        Err(BackendError::InvalidHandle(_))
    ));
    backend.draw(&call(vb2, ib, 1)).unwrap();
}

#[test]
fn live_buffer_count_tracks_leaks() {
    let mut backend = NullBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);
    assert_eq!(
        backend.live_buffers(),
        LiveBuffers {
            vertex: 1,
            index: 1
        }
    );
    backend.destroy_vertex_buffer(vb).unwrap();
    backend.destroy_index_buffer(ib).unwrap();
    assert_eq!(backend.live_buffers(), LiveBuffers::default());
}

#[test]
fn writes_are_bounds_checked() {
    let mut backend = NullBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);

    backend
        .write_index_buffer(ib, 4, &2u16.to_le_bytes())
        .unwrap();
    assert_eq!(backend.index_data(ib).unwrap(), [0, 0, 1, 0, 2, 0]);
    assert_eq!(
        backend.write_vertex_buffer(vb, 32, &[0; 8]),
        Err(BackendError::WriteOutOfBounds {
            offset: 32,
            len: 8,
            size: 36
        })
    );
    assert!(backend.write_vertex_buffer(vb, u64::MAX, &[0]).is_err());
}

#[test]
fn initial_data_must_match_size() {
    let mut backend = NullBackend::new();
    let desc = IndexBufferDesc {
        format: IndexFormat::U32,
        index_count: 2,
        usage: BufferUsage::Dynamic,
    };
    assert_eq!(
        backend.create_index_buffer(&desc, Some(&[0; 4])),
        Err(BackendError::InitialDataSize {
            expected: 8,
            actual: 4
        })
    );
    let ib = backend.create_index_buffer(&desc, None).unwrap();
    assert_eq!(backend.index_data(ib).unwrap(), [0; 8]);
}

#[test]
fn draws_reading_past_buffers_are_rejected() {
    let mut backend = NullBackend::new();
    let (vb, ib) = triangle_buffers(&mut backend);
    backend.begin_frame(&View::default()).unwrap();

    let mut c = call(vb, ib, 1);
    c.geometry = Geometry::Vertices { first: 1, count: 3 };
    assert!(matches!(
        backend.draw(&c),
        Err(BackendError::DrawOutOfBounds { available: 3, .. })
    ));

    c.geometry = Geometry::Indexed {
        buffer: ib,
        first: 1,
        count: 3,
        base_vertex: 0,
    };
    assert!(matches!(
        backend.draw(&c),
        Err(BackendError::DrawOutOfBounds { available: 3, .. })
    ));

    // Index 2 plus base vertex 1 points at vertex 3 of 3.
    c.geometry = Geometry::Indexed {
        buffer: ib,
        first: 0,
        count: 3,
        base_vertex: 1,
    };
    assert!(matches!(
        backend.draw(&c),
        Err(BackendError::DrawOutOfBounds { available: 3, .. })
    ));

    // A negative base vertex that lands every index in range is fine.
    backend
        .write_index_buffer(ib, 0, &[1, 0, 2, 0, 3, 0])
        .unwrap();
    c.geometry = Geometry::Indexed {
        buffer: ib,
        first: 0,
        count: 3,
        base_vertex: -1,
    };
    backend.draw(&c).unwrap();
}
