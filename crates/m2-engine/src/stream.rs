//! Streaming the island around the camera.
//!
//! A [`Loader`] thread owns its own handle on the archive and decodes
//! terrain, models and textures into CPU-side data; [`World`] decides what
//! is wanted around the camera each frame, keeps a few requests in flight
//! (nearest first, so priorities follow the camera), and uploads finished
//! loads within a per-frame byte budget so loading never stalls a frame for
//! long.
//!
//! What is drawn: the low-resolution tiles everywhere (the whole island is
//! always visible), full-detail cells near the camera in place of their
//! tiles, placed models within the object distance, and a sea plane.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use m2_math::glam::{Mat4, Vec3};
use m2_render::{
    BackendError, BlendMode, BufferUsage, DrawCall, DrawQueue, Geometry, IndexBufferDesc,
    IndexBufferHandle, IndexFormat, Layer, MaterialDesc, MaterialId, Primitive, RenderBackend,
    Topology, VertexBufferDesc, VertexBufferHandle,
};

use crate::assets::{AssetError, DecodedTexture, Install};
use crate::model::{CpuModel, GpuModel, MaterialCache};
use crate::world::{self, TerrainMesh, WorldIndex};

/// Something for the loader to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Request {
    Cell(u32),
    Tile(u32),
    Model(u32),
}

/// What the loader sends back.
#[derive(Debug)]
pub enum Loaded {
    Terrain(Request, Result<TerrainMesh, AssetError>),
    Model(u32, Result<CpuModel, AssetError>),
    /// Sent before the terrain or model that first uses it.
    Texture(u32, Option<DecodedTexture>),
}

impl Loaded {
    /// Rough size in bytes, for the upload budget.
    fn size(&self) -> usize {
        match self {
            Self::Terrain(_, Ok(m)) => m.vertices.len() + m.indices.len() * 4,
            Self::Model(_, Ok(m)) => m
                .pieces
                .iter()
                .map(|p| p.vertices.len() + p.indices.len())
                .sum(),
            Self::Texture(_, Some(t)) => t.mips.iter().map(Vec::len).sum(),
            _ => 0,
        }
    }
}

/// Texture widths the loader reads.
#[derive(Clone, Copy, Debug)]
pub struct TextureLimits {
    pub models: u32,
    pub terrain: u32,
}

impl Default for TextureLimits {
    fn default() -> Self {
        Self {
            models: 1024,
            terrain: 2048,
        }
    }
}

/// The loader thread.
#[derive(Debug)]
pub struct Loader {
    requests: Option<Sender<Request>>,
    results: Receiver<Loaded>,
    thread: Option<JoinHandle<()>>,
}

impl Loader {
    pub fn spawn(
        install: &Install,
        index: Arc<WorldIndex>,
        limits: TextureLimits,
    ) -> Result<Self, AssetError> {
        let mut source = install.open_world()?;
        let (req_tx, req_rx) = channel::<Request>();
        let (res_tx, res_rx) = channel::<Loaded>();
        let thread = std::thread::Builder::new()
            .name("asset loader".into())
            .spawn(move || {
                let mut sent_textures = HashSet::new();
                let mut texture = |source: &mut crate::AssetSource<_>, hash: u32, max: u32| {
                    if sent_textures.insert(hash) {
                        let t = source.texture(hash, max).ok();
                        res_tx.send(Loaded::Texture(hash, t)).is_ok()
                    } else {
                        true
                    }
                };
                for request in req_rx {
                    let loaded = match request {
                        Request::Cell(h) | Request::Tile(h) => {
                            let centre = match request {
                                Request::Cell(_) => index.cells.get(&h),
                                _ => index.tiles.get(&h),
                            }
                            .copied()
                            .unwrap_or_default();
                            let mesh = match request {
                                Request::Cell(_) => world::terrain_cell(&mut source, h, centre),
                                _ => world::low_res_tile(&mut source, h, centre),
                            };
                            if let Ok(TerrainMesh {
                                colour_map: Some(map),
                                ..
                            }) = &mesh
                                && !texture(&mut source, *map, limits.terrain)
                            {
                                return;
                            }
                            Loaded::Terrain(request, mesh)
                        }
                        Request::Model(h) => {
                            let model = source
                                .model(h)
                                .and_then(|d| Ok(CpuModel::build(h, &d.parse()?, 0)));
                            if let Ok(m) = &model {
                                for t in m.texture_hashes().collect::<Vec<_>>() {
                                    if !texture(&mut source, t, limits.models) {
                                        return;
                                    }
                                }
                            }
                            Loaded::Model(h, model)
                        }
                    };
                    if res_tx.send(loaded).is_err() {
                        return;
                    }
                }
            })
            .map_err(|source| AssetError::Io {
                path: "asset loader thread".into(),
                source,
            })?;
        Ok(Self {
            requests: Some(req_tx),
            results: res_rx,
            thread: Some(thread),
        })
    }

    pub fn request(&self, r: Request) {
        if let Some(tx) = &self.requests {
            let _ = tx.send(r);
        }
    }

    pub fn try_recv(&self) -> Option<Loaded> {
        self.results.try_recv().ok()
    }
}

impl Drop for Loader {
    fn drop(&mut self) {
        // Closing the request channel ends the thread's loop.
        self.requests = None;
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// How much of the world to keep loaded and drawn.
#[derive(Clone, Copy, Debug)]
pub struct WorldSettings {
    /// Full-detail terrain within this many metres.
    pub detail_radius: f32,
    /// Placed models within this many metres.
    pub object_distance: f32,
    /// Bytes uploaded to the GPU per frame at most (at least one load
    /// always goes through).
    pub upload_budget: usize,
    /// Requests the loader works on at once; fewer keeps priorities fresh.
    pub in_flight: usize,
    /// Sea surface height. An estimate: towns and coastal flats sit at about
    /// -33 m; where the game stores it is not known yet.
    pub sea_level: f32,
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            detail_radius: 600.0,
            object_distance: 1200.0,
            upload_budget: 24 << 20,
            in_flight: 6,
            sea_level: -35.0,
        }
    }
}

#[derive(Debug)]
struct TerrainGpu {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    count: u32,
    material: MaterialId,
    origin: Vec3,
}

#[derive(Debug)]
enum Slot<T> {
    Loading,
    Ready(T),
    Failed,
}

/// Grid cell size for finding placed models near the camera.
const BUCKET: f32 = 200.0;

fn bucket(x: f32, z: f32) -> (i32, i32) {
    ((x / BUCKET).floor() as i32, (z / BUCKET).floor() as i32)
}

/// Load and draw counts, for the HUD and logs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WorldStats {
    pub tiles: usize,
    pub cells: usize,
    pub models: usize,
    pub objects_drawn: usize,
    pub pending: usize,
    pub failed: usize,
}

/// The streamed island.
#[derive(Debug)]
pub struct World {
    pub index: Arc<WorldIndex>,
    pub settings: WorldSettings,
    loader: Loader,
    pending: Vec<Loaded>,
    in_flight: HashSet<Request>,
    tiles: HashMap<u32, Slot<TerrainGpu>>,
    cells: HashMap<u32, Slot<TerrainGpu>>,
    models: HashMap<u32, Slot<GpuModel>>,
    objects: HashMap<(i32, i32), Vec<usize>>,
    terrain_materials: HashMap<Option<u32>, MaterialId>,
    sea: (VertexBufferHandle, IndexBufferHandle, MaterialId),
    stats: WorldStats,
}

impl World {
    /// Indexes the island and starts the loader. Reading the layers takes a
    /// few seconds.
    pub fn open(
        install: &Install,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        settings: WorldSettings,
    ) -> Result<Self, AssetError> {
        let mut source = install.open_world()?;
        let index = Arc::new(WorldIndex::build(&mut source)?);
        drop(source);
        let loader = Loader::spawn(install, index.clone(), TextureLimits::default())?;
        let mut objects: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        for (i, o) in index.objects.iter().enumerate() {
            objects
                .entry(bucket(o.position.x, o.position.z))
                .or_default()
                .push(i);
        }
        let sea = sea_plane(backend, materials, settings.sea_level)?;
        let mut world = Self {
            index,
            settings,
            loader,
            pending: Vec::new(),
            in_flight: HashSet::new(),
            tiles: HashMap::new(),
            cells: HashMap::new(),
            models: HashMap::new(),
            objects,
            terrain_materials: HashMap::new(),
            sea,
            stats: WorldStats::default(),
        };
        // Every tile, so the whole island shows from the start.
        let mut tiles: Vec<u32> = world.index.tiles.keys().copied().collect();
        tiles.sort_unstable();
        for t in tiles {
            world.tiles.insert(t, Slot::Loading);
            world.loader.request(Request::Tile(t));
        }
        Ok(world)
    }

    pub fn stats(&self) -> WorldStats {
        self.stats
    }

    /// Requests what `eye` needs, drops what it no longer does, and uploads
    /// finished loads.
    pub fn update(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        eye: Vec3,
    ) -> Result<(), BackendError> {
        self.receive(backend, materials)?;

        // Cells: nearest first, a few at a time.
        let wanted = self
            .index
            .cells_near(eye.x, eye.z, self.settings.detail_radius);
        let wanted_set: HashSet<u32> = wanted.iter().map(|c| c.0).collect();
        let keep = self.settings.detail_radius + 300.0;
        let far: Vec<u32> = self
            .cells
            .keys()
            .copied()
            .filter(|h| {
                !wanted_set.contains(h)
                    && self
                        .index
                        .cells
                        .get(h)
                        .is_none_or(|&c| world::square_distance(c, eye.x, eye.z) > keep)
            })
            .collect();
        for h in far {
            if let Some(Slot::Ready(t)) = self.cells.remove(&h) {
                backend.destroy_vertex_buffer(t.vertex_buffer)?;
                backend.destroy_index_buffer(t.index_buffer)?;
            }
        }
        let mut requests: Vec<(f32, Request)> = wanted
            .iter()
            .filter(|(h, _)| !self.cells.contains_key(h))
            .map(|&(h, c)| (world::square_distance(c, eye.x, eye.z), Request::Cell(h)))
            .collect();

        // Models of placed objects in range.
        let reach = self.settings.object_distance;
        let (b0, b1) = (
            bucket(eye.x - reach, eye.z - reach),
            bucket(eye.x + reach, eye.z + reach),
        );
        for bx in b0.0..=b1.0 {
            for bz in b0.1..=b1.1 {
                for &i in self.objects.get(&(bx, bz)).map_or(&[][..], Vec::as_slice) {
                    let o = &self.index.objects[i];
                    if self.models.contains_key(&o.model) {
                        continue;
                    }
                    let d = o.position.distance(eye);
                    if d <= reach {
                        requests.push((d, Request::Model(o.model)));
                    }
                }
            }
        }
        requests.sort_by(|a, b| a.0.total_cmp(&b.0));
        for (_, r) in requests {
            if self.in_flight.len() >= self.settings.in_flight {
                break;
            }
            if self.in_flight.insert(r) {
                match r {
                    Request::Cell(h) => drop(self.cells.insert(h, Slot::Loading)),
                    Request::Model(h) => drop(self.models.insert(h, Slot::Loading)),
                    Request::Tile(h) => drop(self.tiles.insert(h, Slot::Loading)),
                }
                self.loader.request(r);
            }
        }
        self.stats.pending = self.in_flight.len()
            + self.pending.len()
            + self
                .tiles
                .values()
                .filter(|t| matches!(t, Slot::Loading))
                .count();
        Ok(())
    }

    /// Uploads finished loads within the frame's budget.
    fn receive(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
    ) -> Result<(), BackendError> {
        while let Some(l) = self.loader.try_recv() {
            self.pending.push(l);
        }
        let mut spent = 0;
        let mut taken = 0;
        for l in &self.pending {
            if taken > 0 && spent + l.size() > self.settings.upload_budget {
                break;
            }
            spent += l.size();
            taken += 1;
        }
        let batch: Vec<Loaded> = self.pending.drain(..taken).collect();
        for l in batch {
            match l {
                Loaded::Texture(hash, t) => materials.add_texture(backend, hash, t.as_ref())?,
                Loaded::Terrain(request, mesh) => {
                    self.in_flight.remove(&request);
                    let slot = match mesh {
                        Ok(m) if !m.indices.is_empty() => {
                            Slot::Ready(self.upload_terrain(backend, materials, &m)?)
                        }
                        Ok(_) => Slot::Failed,
                        Err(e) => {
                            eprintln!("terrain: {e}");
                            Slot::Failed
                        }
                    };
                    let table = match request {
                        Request::Cell(_) => &mut self.cells,
                        _ => &mut self.tiles,
                    };
                    let (Request::Cell(h) | Request::Tile(h) | Request::Model(h)) = request;
                    // A cell dropped while loading is not wanted any more.
                    match (table.get(&h), slot) {
                        (Some(Slot::Loading), slot) => {
                            table.insert(h, slot);
                        }
                        (_, Slot::Ready(t)) => {
                            backend.destroy_vertex_buffer(t.vertex_buffer)?;
                            backend.destroy_index_buffer(t.index_buffer)?;
                        }
                        _ => {}
                    }
                }
                Loaded::Model(hash, model) => {
                    self.in_flight.remove(&Request::Model(hash));
                    let slot = match model {
                        Ok(m) if !m.pieces.is_empty() => {
                            Slot::Ready(GpuModel::upload(backend, materials, &m)?)
                        }
                        Ok(_) => Slot::Failed,
                        Err(e) => {
                            eprintln!("model {hash:08x}: {e}");
                            Slot::Failed
                        }
                    };
                    self.models.insert(hash, slot);
                }
            }
        }
        fn ready<T>(slots: &HashMap<u32, Slot<T>>) -> usize {
            slots
                .values()
                .filter(|s| matches!(s, Slot::Ready(_)))
                .count()
        }
        fn failed<T>(slots: &HashMap<u32, Slot<T>>) -> usize {
            slots.values().filter(|s| matches!(s, Slot::Failed)).count()
        }
        self.stats.tiles = ready(&self.tiles);
        self.stats.cells = ready(&self.cells);
        self.stats.models = ready(&self.models);
        self.stats.failed = failed(&self.tiles) + failed(&self.cells) + failed(&self.models);
        Ok(())
    }

    fn upload_terrain(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        mesh: &TerrainMesh,
    ) -> Result<TerrainGpu, BackendError> {
        let material = match self.terrain_materials.get(&mesh.colour_map) {
            Some(&m) => m,
            None => {
                let texture = mesh.colour_map.and_then(|t| materials.texture(t));
                let m = materials.define(
                    backend,
                    &MaterialDesc {
                        texture: texture.map(|t| t.0),
                        color: if texture.is_some() {
                            [1.0; 4]
                        } else {
                            [0.32, 0.42, 0.22, 1.0]
                        },
                        ..MaterialDesc::default()
                    },
                )?;
                self.terrain_materials.insert(mesh.colour_map, m);
                m
            }
        };
        Ok(TerrainGpu {
            vertex_buffer: backend
                .create_vertex_buffer(&mesh.vertex_desc(), Some(&mesh.vertices))?,
            index_buffer: backend
                .create_index_buffer(&mesh.index_desc(), Some(&mesh.index_bytes()))?,
            count: mesh.indices.len() as u32,
            material,
            origin: mesh.origin,
        })
    }

    /// Queues terrain, sea and the placed models in range of `eye`.
    pub fn draw(&mut self, queue: &mut DrawQueue, eye: Vec3) {
        let terrain = |t: &TerrainGpu, queue: &mut DrawQueue| {
            queue.push(Primitive {
                call: DrawCall {
                    material: t.material,
                    topology: Topology::TriangleList,
                    vertex_buffer: t.vertex_buffer,
                    geometry: Geometry::Indexed {
                        buffer: t.index_buffer,
                        first: 0,
                        count: t.count,
                        base_vertex: 0,
                    },
                    world: Mat4::from_translation(t.origin),
                    skin: None,
                },
                layer: Layer::WORLD,
                translucent: false,
                sort_origin: t.origin,
                user: 0,
            });
        };
        let detailed: Vec<Vec3> = self
            .cells
            .values()
            .filter_map(|s| match s {
                Slot::Ready(t) => {
                    terrain(t, queue);
                    Some(t.origin)
                }
                _ => None,
            })
            .collect();
        for slot in self.tiles.values() {
            if let Slot::Ready(t) = slot {
                let covered = detailed
                    .iter()
                    .any(|c| (c.x - t.origin.x).abs() < 1.0 && (c.z - t.origin.z).abs() < 1.0);
                if !covered {
                    terrain(t, queue);
                }
            }
        }

        let reach = self.settings.object_distance;
        let (b0, b1) = (
            bucket(eye.x - reach, eye.z - reach),
            bucket(eye.x + reach, eye.z + reach),
        );
        let mut drawn = 0;
        for bx in b0.0..=b1.0 {
            for bz in b0.1..=b1.1 {
                for &i in self.objects.get(&(bx, bz)).map_or(&[][..], Vec::as_slice) {
                    let o = &self.index.objects[i];
                    if o.position.distance(eye) > reach {
                        continue;
                    }
                    if let Some(Slot::Ready(m)) = self.models.get(&o.model) {
                        m.draw(
                            queue,
                            Mat4::from_rotation_translation(o.rotation, o.position),
                        );
                        drawn += 1;
                    }
                }
            }
        }
        self.stats.objects_drawn = drawn;

        let (vb, ib, material) = self.sea;
        // Follows the camera so its edge is never in view.
        let centre = Vec3::new(eye.x, self.settings.sea_level, eye.z);
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
                world: Mat4::from_translation(Vec3::new(eye.x, 0.0, eye.z)),
                skin: None,
            },
            layer: Layer::WORLD,
            translucent: true,
            sort_origin: centre,
            user: 0,
        });
    }
}

/// A 20 km square of translucent water at `level`, centred on the origin.
fn sea_plane(
    backend: &mut dyn RenderBackend,
    materials: &mut MaterialCache,
    level: f32,
) -> Result<(VertexBufferHandle, IndexBufferHandle, MaterialId), BackendError> {
    let h = 10_000.0f32;
    let vertices: Vec<u8> = [(-h, h), (h, h), (h, -h), (-h, -h)]
        .iter()
        .flat_map(|&(x, z)| [x, level, z, 0.0, 1.0, 0.0, 0.0, 0.0])
        .flat_map(f32::to_le_bytes)
        .collect();
    let vb = backend.create_vertex_buffer(
        &VertexBufferDesc {
            layout: TerrainMesh::layout(),
            vertex_count: 4,
            usage: BufferUsage::Static,
        },
        Some(&vertices),
    )?;
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
    let material = materials.define(
        backend,
        &MaterialDesc {
            color: [0.10, 0.30, 0.42, 0.75],
            blend: BlendMode::Translucent,
            double_sided: true,
            ..MaterialDesc::default()
        },
    )?;
    Ok((vb, ib, material))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_floor_negative_coordinates() {
        assert_eq!(bucket(0.0, 0.0), (0, 0));
        assert_eq!(bucket(199.0, -1.0), (0, -1));
        assert_eq!(bucket(-200.0, 400.0), (-1, 2));
    }
}
