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
//!
//! A town is drawn as merged batches (`TINY` models), but collides
//! building by building: each batch names the layer entity of each of its
//! buildings, that entity names its own model, and that model's `PHY2`
//! goes where the entity stands. A building whose model isn't found
//! collides through its own triangles of the batch.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;

use m2_math::glam::{Mat4, Quat, Vec3, Vec4, Vec4Swizzles};
use m2_render::{
    BackendError, BlendMode, BufferUsage, DetailMap, DrawCall, DrawQueue, Geometry,
    IndexBufferDesc, IndexBufferHandle, IndexFormat, Layer, MaterialDesc, MaterialId, Primitive,
    Reflection, RenderBackend, Specular, TerrainLayer, Topology, VertexBufferDesc,
    VertexBufferHandle,
};

use m2_assets::model::ModelInfo;
use m2_assets::{Model, Ucfx};

use m2_assets::model::MODEL_TYPE;

use crate::assets::{AssetError, DecodedTexture, Install};
use crate::model::{CpuModel, GpuModel, MaterialCache, MemberShape, flags};
use crate::physics::{CollisionShape, DebrisId, Loose, Physics, StaticKey};
use crate::scatter::{self, CellScatter};
use crate::world::{self, TerrainMesh, TerrainSurface, WorldIndex};

/// Something for the loader to read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Request {
    Cell(u32),
    Tile(u32),
    Model(u32),
    /// A terrain cell's ground scatter.
    Scatter(u32),
}

/// What the loader sends back.
#[derive(Debug)]
pub enum Loaded {
    Terrain(Request, Result<TerrainMesh, AssetError>),
    Model(u32, Result<CpuModel, AssetError>),
    Scatter(u32, Result<CellScatter, AssetError>),
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
            Self::Scatter(_, Ok(s)) => s.size(),
            _ => 0,
        }
    }
}

/// Texture widths the loader reads.
#[derive(Clone, Copy, Debug)]
pub struct TextureLimits {
    pub models: u32,
    pub terrain: u32,
    pub scatter: u32,
}

impl Default for TextureLimits {
    fn default() -> Self {
        Self {
            models: 2048,
            terrain: 2048,
            scatter: 256,
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
                let mut building_shapes = HashMap::new();
                let mut texture =
                    |source: &mut crate::AssetSource<_>, hash: u32, max: u32, linear: bool| {
                        if sent_textures.insert(hash) {
                            let t = match source.texture(hash, max) {
                                Ok(mut t) => {
                                    t.linear = linear;
                                    Some(t)
                                }
                                Err(e) => {
                                    eprintln!("texture {hash:08x}: {e}");
                                    None
                                }
                            };
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
                            if let Ok(m) = &mesh {
                                for s in &m.surfaces {
                                    for (t, linear, map) in s.textures() {
                                        let max = if map { limits.terrain } else { limits.models };
                                        if !texture(&mut source, t, max, linear) {
                                            return;
                                        }
                                    }
                                }
                            }
                            Loaded::Terrain(request, mesh)
                        }
                        Request::Model(h) => {
                            let model = source.model(h).and_then(|d| {
                                let mut m = CpuModel::from_data(&d, 0)?;
                                let parsed = d.parse()?;
                                if parsed.info.flags & ModelInfo::FLAG_TINY != 0 {
                                    m.collision = None;
                                    m.members = batch_members(
                                        &mut source,
                                        &index,
                                        &parsed,
                                        &mut building_shapes,
                                    );
                                }
                                Ok(m)
                            });
                            if let Ok(m) = &model {
                                for (t, linear) in m.texture_requests() {
                                    if !texture(&mut source, t, limits.models, linear) {
                                        return;
                                    }
                                }
                            }
                            Loaded::Model(h, model)
                        }
                        Request::Scatter(h) => {
                            let centre = index.cells.get(&h).copied().unwrap_or_default();
                            let found = scatter::cell_scatter(&mut source, h, centre);
                            if let Ok(s) = &found {
                                for t in s.meshes.iter().filter_map(|m| m.texture()) {
                                    if !texture(&mut source, t, limits.scatter, false) {
                                        return;
                                    }
                                }
                            }
                            Loaded::Scatter(h, found)
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

/// The collision of a merged batch's buildings: each building's own
/// model's shape at the building's place, else its triangles of the
/// batch. `shapes` caches the buildings' models' shapes between batches.
fn batch_members<R: std::io::Read + std::io::Seek>(
    source: &mut crate::AssetSource<R>,
    index: &WorldIndex,
    batch: &Model<'_>,
    shapes: &mut HashMap<u32, Option<CollisionShape>>,
) -> Vec<MemberShape> {
    let mut out = Vec::new();
    for (i, entity) in batch.tiny_members().into_iter().enumerate() {
        let own = index.members.get(&entity).and_then(|member| {
            member.models.iter().find_map(|&model| {
                let shape = shapes
                    .entry(model)
                    .or_insert_with(|| building_shape(source, model))
                    .clone()?;
                Some(MemberShape {
                    shape,
                    placed: Some((member.position, member.rotation)),
                })
            })
        });
        let shape = own.or_else(|| {
            CollisionShape::for_sub_object(batch, i as u32, 0).map(|shape| MemberShape {
                shape,
                placed: None,
            })
        });
        out.extend(shape);
    }
    out
}

/// The collision of a building's own model, from its resident asset.
fn building_shape<R: std::io::Read + std::io::Seek>(
    source: &mut crate::AssetSource<R>,
    model: u32,
) -> Option<CollisionShape> {
    let base = source.asset(model, MODEL_TYPE).ok()?;
    let parsed = Model::parse(&base).ok()?;
    let phy2 = Ucfx::parse(&base).ok().and_then(|u| u.top_level(*b"PHY2"));
    CollisionShape::for_model(&parsed, phy2, 0)
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
    /// Sea surface height. The game's water vertex shaders place the sea
    /// at -36 m (a constant in every one of them), where the layers' water
    /// mines and boats float too.
    pub sea_level: f32,
    /// Grass and other ground scatter within this many metres (each clump
    /// mesh also stops at its own fade distance).
    pub scatter_distance: f32,
    /// Props placed by name (rocks, plants, fences, sandbags: tens of
    /// thousands) within this many metres at most; each is also drawn no
    /// further than [`PROP_SIZES`] times its size.
    pub prop_distance: f32,
}

/// A prop placed by name is drawn within this many times its size, so a
/// 2 m rock stops at 160 m and a 10 m fence at 800 m.
pub const PROP_SIZES: f32 = 80.0;

/// How far a prop with `bounds` is drawn, at most `limit`.
fn draw_reach(bounds: (Vec3, Vec3), limit: f32) -> f32 {
    ((bounds.1 - bounds.0).length() * PROP_SIZES).min(limit)
}

impl Default for WorldSettings {
    fn default() -> Self {
        Self {
            // Models reach past the full-detail ground, so the skyline
            // (towers, bridges, pylons) stays in view.
            detail_radius: 1000.0,
            object_distance: 2000.0,
            upload_budget: 24 << 20,
            in_flight: 6,
            sea_level: -36.0,
            scatter_distance: 150.0,
            prop_distance: 800.0,
        }
    }
}

#[derive(Debug)]
struct TerrainGpu {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    /// First index, index count, material, and whether it blends.
    ranges: Vec<(u32, u32, MaterialId, bool)>,
    origin: Vec3,
}

#[derive(Debug)]
struct ClumpGpu {
    vertex_buffer: VertexBufferHandle,
    index_buffer: IndexBufferHandle,
    count: u32,
    material: MaterialId,
    translucent: bool,
    instances: Vec<Mat4>,
}

#[derive(Debug)]
enum Slot<T> {
    Loading,
    Ready(T),
    Failed,
}

/// Metres per repeat of the terrain detail texture. Invented: the game's
/// own scale for it has not been found.
const DETAIL_TILE: f32 = 6.0;

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
    /// Scatter instances loaded, and drawn last frame.
    pub scatter_loaded: usize,
    pub scatter_drawn: usize,
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
    scatter: HashMap<u32, Slot<Vec<ClumpGpu>>>,
    objects: HashMap<(i32, i32), Vec<usize>>,
    /// Placed objects by model hash.
    by_model: HashMap<u32, Vec<usize>>,
    /// Placed objects by the entity placing them, for the members of
    /// merged batches.
    by_entity: HashMap<u32, usize>,
    /// Ground materials by their textures and layer coordinates
    /// ([`ground_key`]).
    terrain_materials: HashMap<Vec<u32>, MaterialId>,
    sea: (VertexBufferHandle, IndexBufferHandle, MaterialId),
    /// The sky's cloud noise, if it loaded.
    pub cloud_noise: Option<m2_render::TextureHandle>,
    stats: WorldStats,
    /// Placed objects (index into the index's `objects`) blown up: drawn
    /// as their wrecks, or not at all without one.
    pub destroyed: HashSet<usize>,
    /// The models of placed objects that come loose when knocked, by hash.
    loose: HashMap<u32, Arc<Loose>>,
    /// Placed objects knocked loose: their pieces, in [`Loose`] order.
    knocked: HashMap<usize, Vec<DebrisId>>,
    /// Full-detail cells and placed models as static colliders. Step it
    /// each tick; colliders added by [`update`](Self::update) join scene
    /// queries on the next step.
    pub physics: Physics,
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
        // The sea's shore foam, the one texture the game's water shader
        // takes.
        let foam = m2_assets::name_hash("global_seafoam01");
        match source.texture(foam, 512) {
            Ok(t) => materials.add_texture(backend, foam, Some(&t))?,
            Err(e) => eprintln!("sea foam texture {foam:08x}: {e}"),
        }
        // The noise the game's cloud shaders build the sky's clouds from.
        let noise = m2_assets::name_hash("cloud_noise");
        match source.texture(noise, 512) {
            Ok(mut t) => {
                t.linear = true;
                materials.add_texture(backend, noise, Some(&t))?;
            }
            Err(e) => eprintln!("cloud noise texture {noise:08x}: {e}"),
        }
        let cloud_noise = materials.texture(noise).map(|(t, _)| t);
        drop(source);
        let loader = Loader::spawn(install, index.clone(), TextureLimits::default())?;
        let mut objects: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let mut by_model: HashMap<u32, Vec<usize>> = HashMap::new();
        let mut by_entity: HashMap<u32, usize> = HashMap::new();
        for (i, o) in index.objects.iter().enumerate() {
            by_entity.insert(o.entity, i);
            objects
                .entry(bucket(o.position.x, o.position.z))
                .or_default()
                .push(i);
            by_model.entry(o.model).or_default().push(i);
        }
        let foam = materials.texture(foam).map(|(t, _)| t);
        let sea = sea_plane(backend, materials, settings.sea_level, foam)?;
        let mut world = Self {
            index,
            settings,
            loader,
            pending: Vec::new(),
            in_flight: HashSet::new(),
            tiles: HashMap::new(),
            cells: HashMap::new(),
            models: HashMap::new(),
            scatter: HashMap::new(),
            objects,
            by_model,
            by_entity,
            terrain_materials: HashMap::new(),
            sea,
            cloud_noise,
            stats: WorldStats::default(),
            destroyed: HashSet::new(),
            loose: HashMap::new(),
            knocked: HashMap::new(),
            physics: Physics::new(),
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

    /// The mass to shift (kg) to knock placed object `i` loose; `None`
    /// unless its model comes loose ([`Loose`]) and it stands.
    pub fn loose_mass(&self, i: usize) -> Option<f32> {
        if self.knocked.contains_key(&i)
            || self.destroyed.contains(&i)
            || !self.physics.has_static(StaticKey::Object(i))
        {
            return None;
        }
        self.loose
            .get(&self.index.objects.get(i)?.model)
            .map(|l| l.mass)
    }

    /// Knocks placed object `i` loose: its standing collision goes and its
    /// pieces fly off at `velocity` (m/s), turning at `spin` (radians a
    /// second about each axis). False unless it came loose.
    pub fn knock(&mut self, i: usize, velocity: Vec3, spin: Vec3) -> bool {
        if self.loose_mass(i).is_none() {
            return false;
        }
        let o = &self.index.objects[i];
        let Some(loose) = self.loose.get(&o.model).cloned() else {
            return false;
        };
        self.physics.remove_static(StaticKey::Object(i));
        let placed = Mat4::from_rotation_translation(o.rotation, o.position);
        let pieces = loose
            .pieces
            .iter()
            .zip(loose.starts(placed))
            .map(|(piece, at)| self.physics.add_debris(piece, at, velocity, spin))
            .collect();
        self.knocked.insert(i, pieces);
        true
    }

    /// Placed objects standing that come loose with any of their model
    /// within `reach` metres of `at` (as far as its box's middle less
    /// half its size), with how far.
    pub fn loose_near(&self, at: Vec3, reach: f32) -> Vec<(usize, f32)> {
        let far = reach + BUCKET;
        let (b0, b1) = (
            bucket(at.x - far, at.z - far),
            bucket(at.x + far, at.z + far),
        );
        let mut out = Vec::new();
        for bx in b0.0..=b1.0 {
            for bz in b0.1..=b1.1 {
                for &i in self.objects.get(&(bx, bz)).map_or(&[][..], Vec::as_slice) {
                    let o = &self.index.objects[i];
                    let Some(Slot::Ready(m)) = self.models.get(&o.model) else {
                        continue;
                    };
                    if self.loose_mass(i).is_none() {
                        continue;
                    }
                    let (lo, hi) = m.bounds;
                    let centre = o.position + o.rotation * ((lo + hi) * 0.5);
                    let d = (centre.distance(at) - (hi - lo).length() * 0.5).max(0.0);
                    if d <= reach {
                        out.push((i, d));
                    }
                }
            }
        }
        out
    }

    /// How many placed objects have been knocked loose.
    pub fn knocked(&self) -> usize {
        self.knocked.len()
    }

    /// The placed objects drawn from `eye` (in range, loaded, as they
    /// stand) whose model's box covers `spot` from above, give or take
    /// `pad` metres: each one's model, place and box size. What stands at
    /// a mission's gate besides the gate (round 99's broken gates still
    /// showed an arch and doors).
    pub fn covering(&self, spot: Vec3, eye: Vec3, pad: f32) -> Vec<(u32, Vec3, Vec3)> {
        let (reach, props) = (self.settings.object_distance, self.settings.prop_distance);
        let mut out = Vec::new();
        for (i, o) in self.index.objects.iter().enumerate() {
            let d = o.position.distance(eye);
            let Some(Slot::Ready(m)) = self.models.get(&o.model) else {
                continue;
            };
            if d > if o.named {
                draw_reach(m.bounds, props)
            } else {
                reach
            } || self.knocked.contains_key(&i)
                || (self.destroyed.contains(&i) && !m.has_wreck())
            {
                continue;
            }
            let local = o.rotation.inverse() * (spot - o.position);
            let (lo, hi) = m.bounds;
            if local.x >= lo.x - pad
                && local.x <= hi.x + pad
                && local.z >= lo.z - pad
                && local.z <= hi.z + pad
            {
                out.push((o.model, o.position, hi - lo));
            }
        }
        out
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
            self.physics.remove_static(StaticKey::Cell(h));
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

        // Scatter, nearer still.
        let reach = self.settings.scatter_distance;
        let near = self.index.cells_near(eye.x, eye.z, reach);
        let far: Vec<u32> = self
            .scatter
            .keys()
            .copied()
            .filter(|h| {
                self.index
                    .cells
                    .get(h)
                    .is_none_or(|&c| world::square_distance(c, eye.x, eye.z) > reach + 100.0)
            })
            .collect();
        for h in far {
            if let Some(Slot::Ready(clumps)) = self.scatter.remove(&h) {
                destroy_clumps(backend, clumps)?;
            }
        }
        requests.extend(
            near.iter()
                .filter(|(h, _)| !self.scatter.contains_key(h))
                .map(|&(h, c)| {
                    // After the ground under it.
                    let d = world::square_distance(c, eye.x, eye.z) + 50.0;
                    (d, Request::Scatter(h))
                }),
        );

        // Models of placed objects in range.
        let reach = self.settings.object_distance;
        let props = self.settings.prop_distance;
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
                    let reach = if o.named { props } else { reach };
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
                    Request::Scatter(h) => drop(self.scatter.insert(h, Slot::Loading)),
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
                    let collision = mesh
                        .as_ref()
                        .ok()
                        .and_then(|m| m.collision.clone().map(|c| (c, m.origin)));
                    let slot = match mesh {
                        Ok(m) if !m.indices.is_empty() => {
                            let (Request::Cell(h)
                            | Request::Tile(h)
                            | Request::Model(h)
                            | Request::Scatter(h)) = request;
                            let gpu = self.upload_terrain(backend, materials, &m)?;
                            log_terrain(h, &m, &gpu, materials);
                            Slot::Ready(gpu)
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
                    let (Request::Cell(h)
                    | Request::Tile(h)
                    | Request::Model(h)
                    | Request::Scatter(h)) = request;
                    // A cell dropped while loading is not wanted any more.
                    match (table.get(&h), slot) {
                        (Some(Slot::Loading), slot) => {
                            if let (Request::Cell(_), Slot::Ready(_), Some((shape, origin))) =
                                (request, &slot, &collision)
                            {
                                self.physics.add_static(
                                    StaticKey::Cell(h),
                                    shape,
                                    *origin,
                                    Quat::IDENTITY,
                                );
                            }
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
                            if let Some(loose) = m.collision.as_ref().and_then(|c| c.loose.clone())
                            {
                                self.loose.insert(hash, loose);
                            }
                            for &i in self.by_model.get(&hash).map_or(&[][..], Vec::as_slice) {
                                let o = &self.index.objects[i];
                                // A prop placed by name with no collision
                                // of its own (a bush, a plant) has none:
                                // vehicles and people go through it.
                                if let Some(shape) =
                                    m.collision.as_ref().filter(|s| !(o.named && s.stand_in))
                                {
                                    self.physics.add_static(
                                        StaticKey::Object(i),
                                        shape,
                                        o.position,
                                        o.rotation,
                                    );
                                }
                                for (k, member) in m.members.iter().enumerate() {
                                    let (position, rotation) =
                                        member.placed.unwrap_or((o.position, o.rotation));
                                    self.physics.add_static(
                                        StaticKey::Member(i, k),
                                        &member.shape,
                                        position,
                                        rotation,
                                    );
                                }
                            }
                            if !m.tiny_members.is_empty()
                                && std::env::var_os("M2_LOG_BATCHES").is_some()
                            {
                                let own = m
                                    .tiny_members
                                    .iter()
                                    .filter(|e| self.by_entity.contains_key(e))
                                    .count();
                                eprintln!(
                                    "batch {hash:08x}: {} members, {own} placed by their own models",
                                    m.tiny_members.len()
                                );
                            }
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
                Loaded::Scatter(cell, found) => {
                    self.in_flight.remove(&Request::Scatter(cell));
                    let slot = match found {
                        Ok(s) if !s.meshes.is_empty() => {
                            let mut clumps = Vec::with_capacity(s.meshes.len());
                            for m in s.meshes {
                                if m.material.flags & flags::HIDDEN != 0 {
                                    continue;
                                }
                                let material = materials.material(backend, &m.material)?;
                                clumps.push(ClumpGpu {
                                    vertex_buffer: backend.create_vertex_buffer(
                                        &m.vertex_desc(),
                                        Some(&m.vertices),
                                    )?,
                                    index_buffer: backend.create_index_buffer(
                                        &m.index_desc(),
                                        Some(&m.index_bytes()),
                                    )?,
                                    count: m.indices.len() as u32,
                                    material,
                                    translucent: materials.is_translucent(material),
                                    instances: m.instances,
                                });
                            }
                            Slot::Ready(clumps)
                        }
                        Ok(_) => Slot::Failed,
                        Err(e) => {
                            eprintln!("scatter {cell:08x}: {e}");
                            Slot::Failed
                        }
                    };
                    // Dropped while loading: not wanted any more.
                    match (self.scatter.get(&cell), slot) {
                        (Some(Slot::Loading), slot) => drop(self.scatter.insert(cell, slot)),
                        (_, Slot::Ready(clumps)) => destroy_clumps(backend, clumps)?,
                        _ => {}
                    }
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
        self.stats.scatter_loaded = self
            .scatter
            .values()
            .map(|s| match s {
                Slot::Ready(c) => c.iter().map(|c| c.instances.len()).sum(),
                _ => 0,
            })
            .sum();
        self.stats.failed = failed(&self.tiles) + failed(&self.cells) + failed(&self.models);
        Ok(())
    }

    fn upload_terrain(
        &mut self,
        backend: &mut dyn RenderBackend,
        materials: &mut MaterialCache,
        mesh: &TerrainMesh,
    ) -> Result<TerrainGpu, BackendError> {
        let mut ids = Vec::with_capacity(mesh.surfaces.len());
        for surface in &mesh.surfaces {
            ids.push(match surface {
                TerrainSurface::Surface(m) => materials.overlay_material(backend, m)?,
                TerrainSurface::Ground {
                    colour_map,
                    detail_map,
                    layers,
                } => {
                    let key = ground_key(*colour_map, *detail_map, layers);
                    match self.terrain_materials.get(&key) {
                        Some(&m) => m,
                        None => {
                            let m = materials.define(
                                backend,
                                &ground_material(materials, *colour_map, *detail_map, layers),
                            )?;
                            // Kept only once every texture is known.
                            let known = surface
                                .textures()
                                .iter()
                                .all(|&(t, _, _)| materials.has_texture(t));
                            if known {
                                self.terrain_materials.insert(key, m);
                            }
                            m
                        }
                    }
                }
            });
        }
        Ok(TerrainGpu {
            vertex_buffer: backend
                .create_vertex_buffer(&mesh.vertex_desc(), Some(&mesh.vertices))?,
            index_buffer: backend
                .create_index_buffer(&mesh.index_desc(), Some(&mesh.index_bytes()))?,
            // Pieces whose material is never drawn (water masks and the
            // like, as in models) are left out.
            ranges: mesh
                .ranges
                .iter()
                .filter(|&&(_, _, s)| {
                    !matches!(&mesh.surfaces[s],
                        TerrainSurface::Surface(m) if m.flags & flags::HIDDEN != 0)
                })
                .map(|&(first, count, s)| (first, count, ids[s], materials.is_translucent(ids[s])))
                .collect(),
            origin: mesh.origin,
        })
    }

    /// Queues terrain, scatter, sea and the placed models in range of
    /// `eye`. Call inside the backend's frame: scatter stores its instance
    /// transforms on it.
    pub fn draw(
        &mut self,
        backend: &mut dyn RenderBackend,
        queue: &mut DrawQueue,
        eye: Vec3,
        frustum: &Frustum,
    ) -> Result<(), BackendError> {
        let terrain = |t: &TerrainGpu, queue: &mut DrawQueue| {
            for &(first, count, material, translucent) in &t.ranges {
                queue.push(Primitive {
                    call: DrawCall {
                        material,
                        topology: Topology::TriangleList,
                        vertex_buffer: t.vertex_buffer,
                        geometry: Geometry::Indexed {
                            buffer: t.index_buffer,
                            first,
                            count,
                            base_vertex: 0,
                        },
                        world: Mat4::from_translation(t.origin),
                        skin: None,
                        instances: None,
                        uv_offset: [0.0; 2],
                    },
                    layer: Layer::WORLD,
                    translucent,
                    sort_origin: t.origin,
                    user: 0,
                });
            }
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
                // `M2_TILES_UNDER=1` draws them under full-detail cells too,
                // to see what a cell leaves open.
                if !covered || tiles_under() {
                    terrain(t, queue);
                }
            }
        }

        let reach = self.settings.object_distance;
        let props = self.settings.prop_distance;
        let (b0, b1) = (
            bucket(eye.x - reach, eye.z - reach),
            bucket(eye.x + reach, eye.z + reach),
        );
        let mut drawn = 0;
        for bx in b0.0..=b1.0 {
            for bz in b0.1..=b1.1 {
                for &i in self.objects.get(&(bx, bz)).map_or(&[][..], Vec::as_slice) {
                    let o = &self.index.objects[i];
                    let d = o.position.distance(eye);
                    if d > if o.named { props } else { reach } || self.knocked.contains_key(&i) {
                        continue;
                    }
                    if let Some(Slot::Ready(m)) = self.models.get(&o.model)
                        && (!o.named || d <= draw_reach(m.bounds, props))
                        && {
                            // Out of view: not drawn.
                            let (lo, hi) = m.bounds;
                            let centre = o.position + o.rotation * ((lo + hi) * 0.5);
                            frustum.sees(centre, (hi - lo).length() * 0.5)
                        }
                    {
                        let at = Mat4::from_rotation_translation(o.rotation, o.position);
                        if !m.tiny_members.is_empty() {
                            // A merged batch stands in for its members
                            // until their own models draw.
                            let own = |k: u16| {
                                m.tiny_members
                                    .get(usize::from(k))
                                    .and_then(|e| self.by_entity.get(e))
                                    .is_some_and(|&j| j != i && self.member_drawn(j, eye))
                            };
                            m.draw_batch(queue, at, &own);
                            drawn += 1;
                            continue;
                        }
                        let destroyed = self.destroyed.contains(&i);
                        if destroyed && !m.has_wreck() {
                            continue;
                        }
                        m.draw_as(
                            queue,
                            Mat4::from_rotation_translation(o.rotation, o.position),
                            destroyed,
                        );
                        drawn += 1;
                    }
                }
            }
        }
        // Those knocked loose, their pieces where they flew.
        for (&i, pieces) in &self.knocked {
            let o = &self.index.objects[i];
            let (Some(Slot::Ready(m)), Some(loose)) =
                (self.models.get(&o.model), self.loose.get(&o.model))
            else {
                continue;
            };
            let destroyed = self.destroyed.contains(&i);
            if destroyed && !m.has_wreck() {
                continue;
            }
            let places: Vec<Mat4> = pieces
                .iter()
                .filter_map(|&d| self.physics.debris_place(d))
                .collect();
            if places.len() != pieces.len()
                || places
                    .iter()
                    .all(|p| p.w_axis.truncate().distance(eye) > props)
            {
                continue;
            }
            let placed = Mat4::from_rotation_translation(o.rotation, o.position);
            m.draw_moved(
                queue,
                placed,
                &loose.posed(placed, &places),
                loose.breaks || destroyed,
                loose.flying(),
            );
            drawn += 1;
        }
        self.stats.objects_drawn = drawn;
        self.stats.scatter_drawn = self.draw_scatter(backend, queue, eye, frustum)?;

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
                instances: None,
                uv_offset: [0.0; 2],
            },
            layer: Layer::WORLD,
            translucent: true,
            sort_origin: centre,
            user: 0,
        });
        Ok(())
    }

    /// Whether placed object `i` is drawn by its own model from `eye` (in
    /// range and loaded), whether or not it is in view.
    fn member_drawn(&self, i: usize, eye: Vec3) -> bool {
        let o = &self.index.objects[i];
        let d = o.position.distance(eye);
        let props = self.settings.prop_distance;
        match self.models.get(&o.model) {
            Some(Slot::Ready(m)) if m.tiny_members.is_empty() => {
                if o.named {
                    d <= draw_reach(m.bounds, props)
                } else {
                    d <= self.settings.object_distance
                }
            }
            _ => false,
        }
    }

    /// Queues each clump mesh once, instanced at its instances in range
    /// and in view. Returns the instances drawn.
    fn draw_scatter(
        &self,
        backend: &mut dyn RenderBackend,
        queue: &mut DrawQueue,
        eye: Vec3,
        frustum: &Frustum,
    ) -> Result<usize, BackendError> {
        let mut drawn = 0;
        let mut near: Vec<Mat4> = Vec::new();
        for slot in self.scatter.values() {
            let Slot::Ready(clumps) = slot else { continue };
            for c in clumps {
                // Stopping at the meshes' fade distances left the ground
                // nearly bare on the device; until what they mean is
                // checked, everything within reach draws.
                let reach2 = self.settings.scatter_distance.powi(2);
                near.clear();
                near.extend(c.instances.iter().filter(|m| {
                    let at = m.w_axis.truncate();
                    at.distance_squared(eye) <= reach2 && frustum.sees(at, CLUMP_RADIUS)
                }));
                if near.is_empty() {
                    continue;
                }
                drawn += near.len();
                let instances = backend.set_skin(&near)?;
                queue.push(Primitive {
                    call: DrawCall {
                        material: c.material,
                        topology: Topology::TriangleList,
                        vertex_buffer: c.vertex_buffer,
                        geometry: Geometry::Indexed {
                            buffer: c.index_buffer,
                            first: 0,
                            count: c.count,
                            base_vertex: 0,
                        },
                        world: Mat4::IDENTITY,
                        skin: None,
                        instances: Some(instances),
                        uv_offset: [0.0; 2],
                    },
                    layer: Layer::WORLD,
                    translucent: c.translucent,
                    sort_origin: near[0].w_axis.truncate(),
                    user: 0,
                });
            }
        }
        Ok(drawn)
    }
}

/// How far a scatter clump reaches from its origin, metres, for culling
/// (grass and bushes; generous so nothing pops at the screen's edge).
const CLUMP_RADIUS: f32 = 4.0;

fn destroy_clumps(
    backend: &mut dyn RenderBackend,
    clumps: Vec<ClumpGpu>,
) -> Result<(), BackendError> {
    for c in clumps {
        backend.destroy_vertex_buffer(c.vertex_buffer)?;
        backend.destroy_index_buffer(c.index_buffer)?;
    }
    Ok(())
}

/// A ground material's cache key: its textures and the bits of its layer
/// coordinates.
fn ground_key(
    colour_map: Option<u32>,
    detail_map: Option<u32>,
    layers: &[(u32, [[f32; 4]; 2])],
) -> Vec<u32> {
    let mut key = vec![colour_map.unwrap_or(0), detail_map.unwrap_or(0)];
    for (t, rows) in layers {
        key.push(*t);
        key.extend(rows.iter().flatten().map(|f| f.to_bits()));
    }
    key
}

/// How ground is drawn (see [`MaterialDesc::ground`]): the cell's colour
/// map clamped over the cell, the detail map tiled up close, and the tiled
/// layers. Textures not yet loaded are left out.
fn ground_material(
    materials: &MaterialCache,
    colour_map: Option<u32>,
    detail_map: Option<u32>,
    layers: &[(u32, [[f32; 4]; 2])],
) -> MaterialDesc {
    let texture = colour_map.and_then(|t| materials.texture(t));
    let detail = detail_map.and_then(|t| materials.texture(t));
    let mut out = [None; 4];
    // A layer that failed to load would shift the weights onto the wrong
    // textures, so the layers stop at the first missing one.
    for (slot, (t, rows)) in out.iter_mut().zip(layers) {
        match materials.texture(*t) {
            Some((handle, _)) => {
                *slot = Some(TerrainLayer {
                    texture: handle,
                    uv: *rows,
                })
            }
            None => break,
        }
    }
    MaterialDesc {
        texture: texture.map(|t| t.0),
        color: if texture.is_some() {
            [1.0; 4]
        } else {
            [0.32, 0.42, 0.22, 1.0]
        },
        clamp: true,
        detail: detail.map(|(texture, _)| DetailMap {
            texture,
            tile: DETAIL_TILE,
        }),
        layers: out,
        ground: true,
        ..MaterialDesc::default()
    }
}

fn tiles_under() -> bool {
    static UNDER: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *UNDER.get_or_init(|| std::env::var_os("M2_TILES_UNDER").is_some())
}

/// With `M2_LOG_TERRAIN=<hex hash>,...`, prints what a cell or tile
/// draws: each range's surface, material, whether it blends, triangles and
/// height span.
fn log_terrain(hash: u32, mesh: &TerrainMesh, gpu: &TerrainGpu, materials: &MaterialCache) {
    static WANTED: std::sync::OnceLock<HashSet<u32>> = std::sync::OnceLock::new();
    let wanted = WANTED.get_or_init(|| {
        std::env::var("M2_LOG_TERRAIN")
            .map(|v| {
                v.split(',')
                    .filter_map(|h| u32::from_str_radix(h.trim().trim_start_matches("0x"), 16).ok())
                    .collect()
            })
            .unwrap_or_default()
    });
    if !wanted.contains(&hash) {
        return;
    }
    let y = |v: u32| {
        let at = v as usize * world::TERRAIN_STRIDE + 4;
        f32::from_le_bytes(mesh.vertices[at..at + 4].try_into().expect("4 bytes"))
    };
    eprintln!(
        "terrain {hash:08x} at {:?}: {} ranges drawn of {}, {} triangles turned round",
        mesh.origin,
        gpu.ranges.len(),
        mesh.ranges.len(),
        mesh.flipped
    );
    for &(first, count, s) in &mesh.ranges {
        let tris = &mesh.indices[first as usize..(first + count) as usize];
        let (lo, hi) = tris
            .iter()
            .map(|&v| y(v) + mesh.origin.y)
            .fold((f32::MAX, f32::MIN), |(lo, hi), y| (lo.min(y), hi.max(y)));
        let drawn = gpu.ranges.iter().find(|r| r.0 == first);
        let what = match &mesh.surfaces[s] {
            TerrainSurface::Ground {
                colour_map, layers, ..
            } => format!(
                "ground (colour map {colour_map:08x?}, {} layers{})",
                layers.len(),
                colour_map
                    .filter(|&t| !materials.has_texture(t))
                    .map_or("", |_| ", colour map not loaded")
            ),
            TerrainSurface::Surface(m) => format!(
                "surface {:08x} flags {:04x} textures {:08x?}",
                m.name_hash, m.flags, m.textures
            ),
        };
        eprintln!(
            "  {} triangles, y {lo:.1}..{hi:.1}: {what}: {}",
            count / 3,
            drawn.map_or("not drawn".to_string(), |r| format!(
                "material {:?}{}",
                r.2,
                if r.3 { ", blends" } else { "" }
            ))
        );
    }
}

/// A 20 km square of translucent water at `level`, centred on the origin,
/// with `foam` along its shores.
/// The sea material's constants the game's water shader takes:
/// `constant0.x` (reflection strength), `R0` (reflectance facing the eye),
/// `constant0.z` (glint exponent) and the specular colour (taken grey).
/// The game's water material is not in its archives (PC round L), so
/// these are set from code there; fitted to footage of the night start
/// and a sunset ride (PC round K: the near and far night sea 59 and 13
/// against 52 and 9, the sunset glare 152 against 120 to 190, a dimmer
/// mirrored sky darkening the far sea as the game's does). Against the PC
/// clip's night bay the open sea already matched but the far sea was a
/// third too dark: the mirrored sky raised it 2.4 per 0.1 (PC round Y,
/// 21.5 at 0.3, 28.7 at 0.6, against 31.9), so it is 0.7. `M2_SEA` sets
/// them as
/// `reflection,r0,power,glint[,body[,sky[,waves[,fade]]]]`, `body`
/// scaling the diffuse and ambient colours, `sky` the sky mirrored,
/// `waves` the waves' heights (the game's are set from code too) and
/// `fade` how soon short waves fade with distance (8 drawn so far; 1 if
/// the game's OOWavelengthScale is one over the wavelength).
#[derive(Clone, Copy, Debug, PartialEq)]
struct SeaConstants {
    reflection: f32,
    r0: f32,
    power: f32,
    glint: f32,
    body: f32,
    sky: f32,
    waves: f32,
    fade: f32,
}

impl SeaConstants {
    fn get() -> Self {
        let mut c = Self {
            reflection: 1.0,
            r0: 0.02,
            power: 40.0,
            glint: 1.0,
            body: 0.7,
            sky: 0.7,
            waves: 1.0,
            fade: 8.0,
        };
        if let Ok(v) = std::env::var("M2_SEA") {
            let f: Vec<f32> = v.split(',').filter_map(|s| s.trim().parse().ok()).collect();
            if f.len() >= 4 {
                c.reflection = f[0];
                c.r0 = f[1];
                c.power = f[2];
                c.glint = f[3];
            }
            if let Some(&b) = f.get(4) {
                c.body = b;
            }
            if let Some(&s) = f.get(5) {
                c.sky = s;
            }
            if let Some(&w) = f.get(6) {
                c.waves = w;
            }
            if let Some(&d) = f.get(7) {
                c.fade = d;
            }
        }
        c
    }
}

fn sea_plane(
    backend: &mut dyn RenderBackend,
    materials: &mut MaterialCache,
    level: f32,
    foam: Option<m2_render::TextureHandle>,
) -> Result<(VertexBufferHandle, IndexBufferHandle, MaterialId), BackendError> {
    let sea = SeaConstants::get();
    let h = 10_000.0f32;
    let vertices: Vec<u8> = [(-h, h), (h, h), (h, -h), (-h, -h)]
        .iter()
        .flat_map(|&(x, z)| [x, level, z, 0.0, 1.0, 0.0, 0.0, 0.0])
        .flat_map(f32::to_le_bytes)
        .collect();
    let vb = backend.create_vertex_buffer(
        &VertexBufferDesc {
            layout: world::simple_layout(),
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
            // Deep blue-green; shallows are paler and show the bed.
            color: [0.05, 0.20, 0.30, 1.0],
            texture: foam,
            blend: BlendMode::Translucent,
            double_sided: true,
            water: true,
            specular: Some(Specular {
                color: [sea.glint; 3],
                power: sea.power,
                map: None,
            }),
            reflection: Some(Reflection {
                strength: sea.reflection,
                fresnel: Some(sea.r0),
            }),
            direct: [sea.body; 3],
            ambient: [sea.body; 3],
            water_sky: sea.sky,
            water_waves: sea.waves,
            water_fade: sea.fade,
            ..MaterialDesc::default()
        },
    )?;
    Ok((vb, ib, material))
}

/// The four side planes of a view's frustum, facing in. Near and far are
/// left out: distance limits what is drawn already, and so the planes
/// hold for either depth convention.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum([Vec4; 4]);

impl Frustum {
    /// The frustum of `view_proj` (projection times view).
    pub fn from_view_proj(m: Mat4) -> Self {
        let (r0, r1, r3) = (m.row(0), m.row(1), m.row(3));
        Self([r3 + r0, r3 - r0, r3 + r1, r3 - r1].map(|p| p / p.xyz().length().max(1e-6)))
    }

    /// Everything is in view.
    pub fn everything() -> Self {
        Self([Vec4::new(0.0, 0.0, 0.0, 1.0); 4])
    }

    /// Whether any of the sphere at `centre` of `radius` may be in view.
    pub fn sees(&self, centre: Vec3, radius: f32) -> bool {
        self.0.iter().all(|p| p.xyz().dot(centre) + p.w >= -radius)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frustum_sees_ahead_not_behind() {
        let view = Mat4::look_at_rh(Vec3::ZERO, Vec3::new(0.0, 0.0, -1.0), Vec3::Y);
        let proj = Mat4::perspective_rh(1.0, 1.5, 0.1, 1000.0);
        let f = Frustum::from_view_proj(proj * view);
        assert!(f.sees(Vec3::new(0.0, 0.0, -50.0), 1.0));
        assert!(!f.sees(Vec3::new(0.0, 0.0, 50.0), 1.0));
        assert!(!f.sees(Vec3::new(200.0, 0.0, -50.0), 1.0));
        // Big enough to reach into view from just outside it.
        assert!(f.sees(Vec3::new(60.0, 0.0, -50.0), 40.0));
        assert!(Frustum::everything().sees(Vec3::new(0.0, 0.0, 50.0), 0.0));
    }

    #[test]
    fn buckets_floor_negative_coordinates() {
        assert_eq!(bucket(0.0, 0.0), (0, 0));
        assert_eq!(bucket(199.0, -1.0), (0, -1));
        assert_eq!(bucket(-200.0, 400.0), (-1, 2));
    }
}
