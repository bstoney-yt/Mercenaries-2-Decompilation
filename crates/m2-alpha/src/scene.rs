//! What is drawn: the streamed island, or (with `--model`) a ground grid
//! with models from the install standing on it.

use std::fs::File;
use std::io::BufReader;

use anyhow::Context as _;
use m2_engine::{AssetSource, GpuModel, Install, MaterialCache, World, WorldSettings};
use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BufferUsage, DrawCall, DrawQueue, Geometry, IndexBufferDesc, IndexBufferHandle, IndexFormat,
    Layer, MaterialDesc, MaterialId, Primitive, RenderBackend, TextureDesc, TextureFormat,
    Topology, VertexBufferDesc, VertexBufferHandle, VertexFormat, VertexLayout, VertexSemantic,
    generate_mips,
};

/// Jennifer Mui, the playable mercenary whose model the README's export
/// examples use.
pub const DEFAULT_MODEL: u32 = 0x15d8_9832;

/// Where the island opens: the first town, as in the viewer's examples.
pub const DEFAULT_SPAWN: (f32, f32) = (450.0, -1950.0);

/// Half the ground's width in metres.
const GROUND_HALF: f32 = 1000.0;
/// Metres per repeat of the grid texture.
const GRID_TILE: f32 = 4.0;

#[derive(Debug)]
pub struct Placed {
    pub model: GpuModel,
    pub world: Mat4,
}

#[derive(Debug)]
pub struct Scene {
    /// The grid, drawn when there is no island.
    ground: (VertexBufferHandle, IndexBufferHandle, MaterialId),
    pub world: Option<World>,
    pub models: Vec<Placed>,
    pub materials: MaterialCache,
    pub source: Option<AssetSource<BufReader<File>>>,
    /// Where the camera should start: (eye, target).
    pub focus: (Vec3, Vec3),
}

impl Scene {
    pub fn new(backend: &mut dyn RenderBackend) -> anyhow::Result<Self> {
        let mut materials = MaterialCache::new();
        let ground = ground(backend, &mut materials)?;
        Ok(Self {
            ground,
            world: None,
            models: Vec::new(),
            materials,
            source: None,
            focus: (Vec3::new(0.0, 2.0, 6.0), Vec3::new(0.0, 1.0, 0.0)),
        })
    }

    /// Opens the install's world archive.
    pub fn open(&mut self, install: &Install) -> anyhow::Result<()> {
        let source = install
            .open_world()
            .with_context(|| format!("opening {}", install.world_archive().display()))?;
        self.source = Some(source);
        Ok(())
    }

    /// Loads models `hashes` and stands them in a row on the ground, 3 m
    /// apart, facing the camera (models face `+z`).
    pub fn load_models(
        &mut self,
        backend: &mut dyn RenderBackend,
        hashes: &[u32],
    ) -> anyhow::Result<()> {
        let source = self.source.as_mut().context("no install open")?;
        let mut x = 0.0;
        let mut widest = 2.0f32;
        for &hash in hashes {
            let gpu = GpuModel::load(backend, source, &mut self.materials, hash, 0)?;
            let (lo, hi) = gpu.bounds;
            eprintln!(
                "model {hash:08x}: {} draws, {:.1} x {:.1} x {:.1} m{}",
                gpu.draw_count(),
                hi.x - lo.x,
                hi.y - lo.y,
                hi.z - lo.z,
                if gpu.skipped_groups > 0 {
                    format!(
                        ", {} groups in unsupported vertex formats",
                        gpu.skipped_groups
                    )
                } else {
                    String::new()
                }
            );
            let size = hi - lo;
            widest = widest.max(size.x.max(size.z)).max(size.y);
            // Stand it on the ground, centred on x.
            let offset = Vec3::new(x - (lo.x + hi.x) * 0.5, -lo.y, -(lo.z + hi.z) * 0.5);
            self.models.push(Placed {
                model: gpu,
                world: Mat4::from_translation(offset),
            });
            x += size.x.max(1.0) + 3.0;
        }
        let centre = Vec3::new((x - 3.0).max(0.0) * 0.5, widest * 0.5, 0.0);
        self.focus = (
            // Models face +z, so the camera stands on that side.
            centre + Vec3::new(0.0, widest * 0.3, widest * 1.6 + x * 0.5),
            centre,
        );
        Ok(())
    }

    /// Indexes the island and starts streaming it, with the camera placed
    /// above `(x, z)`.
    pub fn open_world(
        &mut self,
        backend: &mut dyn RenderBackend,
        install: &Install,
        settings: WorldSettings,
        spawn: (f32, f32),
    ) -> anyhow::Result<()> {
        let started = std::time::Instant::now();
        let world = World::open(install, backend, &mut self.materials, settings)
            .context("indexing the island")?;
        eprintln!(
            "island: {} cells, {} tiles, {} placed objects (indexed in {:.1} s)",
            world.index.cells.len(),
            world.index.tiles.len(),
            world.index.objects.len(),
            started.elapsed().as_secs_f32()
        );
        let (x, z) = spawn;
        let y = world.index.rough_height(x, z) + 40.0;
        self.focus = (Vec3::new(x, y, z), Vec3::new(x, y - 20.0, z + 60.0));
        self.world = Some(world);
        Ok(())
    }

    /// Streams around `eye`.
    pub fn update(&mut self, backend: &mut dyn RenderBackend, eye: Vec3) -> anyhow::Result<()> {
        if let Some(world) = &mut self.world {
            world.update(backend, &mut self.materials, eye)?;
        }
        Ok(())
    }

    pub fn draw(&mut self, queue: &mut DrawQueue, eye: Vec3) {
        if let Some(world) = &mut self.world {
            world.draw(queue, eye);
        } else {
            self.draw_ground(queue);
        }
        for p in &self.models {
            p.model.draw(queue, p.world);
        }
    }

    fn draw_ground(&self, queue: &mut DrawQueue) {
        let (vb, ib, material) = self.ground;
        queue.push(Primitive {
            call: DrawCall {
                material,
                topology: Topology::TriangleList,
                vertex_buffer: vb,
                geometry: Geometry::Indexed {
                    buffer: ib,
                    first: 0,
                    count: 6,
                    base_vertex: 0,
                },
                world: Mat4::IDENTITY,
            },
            layer: Layer::WORLD,
            translucent: false,
            sort_origin: Vec3::ZERO,
            user: 0,
        });
    }
}

/// A 2 km square at height 0 with a grid texture.
fn ground(
    backend: &mut dyn RenderBackend,
    materials: &mut MaterialCache,
) -> anyhow::Result<(VertexBufferHandle, IndexBufferHandle, MaterialId)> {
    use VertexFormat::*;
    use VertexSemantic::*;
    let layout =
        VertexLayout::packed(&[(Position, Float3), (Normal, Float3), (TexCoord(0), Float2)])?;
    let (h, t) = (GROUND_HALF, GROUND_HALF / GRID_TILE);
    let corners = [
        (-h, h, 0.0, 0.0),
        (h, h, t, 0.0),
        (h, -h, t, t),
        (-h, -h, 0.0, t),
    ];
    let vertices: Vec<u8> = corners
        .iter()
        .flat_map(|&(x, z, u, v)| [x, 0.0, z, 0.0, 1.0, 0.0, u, v])
        .flat_map(f32::to_le_bytes)
        .collect();
    let vb = backend.create_vertex_buffer(
        &VertexBufferDesc {
            layout,
            vertex_count: 4,
            usage: BufferUsage::Static,
        },
        Some(&vertices),
    )?;
    // Clockwise seen from above.
    let indices: Vec<u8> = [0u16, 1, 2, 0, 2, 3]
        .iter()
        .flat_map(|i| i.to_le_bytes())
        .collect();
    let ib = backend.create_index_buffer(
        &IndexBufferDesc {
            format: IndexFormat::U16,
            index_count: 6,
            usage: BufferUsage::Static,
        },
        Some(&indices),
    )?;
    const SIZE: u32 = 128;
    let texels: Vec<u8> = (0..SIZE * SIZE)
        .flat_map(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let line = x < 2 || y < 2;
            if line {
                [90, 96, 90, 255]
            } else {
                [150, 160, 140, 255]
            }
        })
        .collect();
    let mips = generate_mips(SIZE, SIZE, &texels);
    let mips: Vec<&[u8]> = mips.iter().map(Vec::as_slice).collect();
    let texture = backend.create_texture(
        &TextureDesc {
            width: SIZE,
            height: SIZE,
            format: TextureFormat::Rgba8Srgb,
        },
        &mips,
    )?;
    let material = materials.define(
        backend,
        &MaterialDesc {
            texture: Some(texture),
            ..MaterialDesc::default()
        },
    )?;
    Ok((vb, ib, material))
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_render::NullBackend;

    #[test]
    fn ground_draws_through_the_null_backend() {
        let mut backend = NullBackend::new();
        let mut scene = Scene::new(&mut backend).unwrap();
        let mut queue = DrawQueue::new();
        scene.draw(&mut queue, Vec3::ZERO);
        let view = m2_render::View::default();
        backend.begin_frame(&view).unwrap();
        queue.assign_keys(&view);
        queue.sort().unwrap();
        assert_eq!(queue.submit(&mut backend).unwrap().draws, 1);
        backend.end_frame().unwrap();
        assert_eq!(backend.live_buffers().texture, 1);
    }
}
