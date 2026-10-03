//! The island's layout: where its terrain cells, low-resolution tiles and
//! placed models are, and their terrain decoded into upload-ready meshes.
//!
//! [`WorldIndex::build`] reads only the entity layers (finding them through
//! the archive's asset table, so the rest of the 2.5 GB archive is not
//! decoded) and records the `TerrainObject`, `LowResTerrainObject` and
//! `ModelName` placements. Terrain is read later, cell by cell, by
//! [`terrain_cell`] and [`low_res_tile`].
//!
//! # Provenance
//!
//! Format-from-samples, as `m2-assets`; this module only combines what the
//! parsers there decode, following the glTF exporter's world export.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek};

use m2_assets::layer::LAYER_TYPE;
use m2_assets::terrain::{CELL_SIZE, TerrainGroup, colour_map_uv};
use m2_assets::vertex::DeclUsage;
use m2_assets::{BlockContents, Layer, LowResTile, TerrainCell};
use m2_math::glam::{Quat, Vec3};
use m2_render::{
    BufferUsage, IndexBufferDesc, IndexFormat, VertexBufferDesc, VertexFormat, VertexLayout,
    VertexSemantic,
};

use crate::assets::{AssetError, AssetSource};

/// Side of the island in metres (20 cells of 400 m).
pub const ISLAND: f32 = 8000.0;

/// One placed model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPlacement {
    pub model: u32,
    pub position: Vec3,
    pub rotation: Quat,
}

/// Where everything on the island is.
#[derive(Clone, Debug, Default)]
pub struct WorldIndex {
    /// Terrain cell asset hash to cell centre.
    pub cells: HashMap<u32, Vec3>,
    /// Low-resolution tile asset hash to tile centre.
    pub tiles: HashMap<u32, Vec3>,
    /// Models placed by the static layers (the world as it always is,
    /// without mission states).
    pub objects: Vec<ObjectPlacement>,
}

impl WorldIndex {
    /// Reads every entity layer in the archive.
    pub fn build<R: Read + Seek>(source: &mut AssetSource<R>) -> Result<Self, AssetError> {
        let index = source.archive().index();
        let layer_blocks: HashSet<usize> = index
            .assets()
            .iter()
            .filter(|a| index.type_hash(a) == LAYER_TYPE)
            .map(|a| usize::from(a.block))
            .collect();
        let mut blocks: Vec<(usize, bool)> = layer_blocks
            .into_iter()
            .map(|b| (b, index.blocks()[b].path.contains("layers_static")))
            .collect();
        blocks.sort_unstable();

        let mut world = Self::default();
        for (block, is_static) in blocks {
            let decoded = source.block(block)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash != LAYER_TYPE {
                    continue;
                }
                let Ok(layer) = Layer::parse(data) else {
                    continue;
                };
                world.add_layer(&layer, is_static);
            }
        }
        Ok(world)
    }

    fn add_layer(&mut self, layer: &Layer<'_>, is_static: bool) {
        let placements = layer.placements();
        let at = |entity| placements.get(&entity);
        for (entity, asset) in layer.hashes("TerrainObject") {
            if let Some(p) = at(entity) {
                self.cells.insert(asset, Vec3::from(p.position));
            }
        }
        for (entity, asset) in layer.hashes("LowResTerrainObject") {
            if let Some(p) = at(entity) {
                self.tiles.insert(asset, Vec3::from(p.position));
            }
        }
        if is_static {
            for (entity, model) in layer.hashes("ModelName") {
                if let Some(p) = at(entity) {
                    let rotation = Quat::from_array(p.rotation);
                    self.objects.push(ObjectPlacement {
                        model,
                        position: Vec3::from(p.position),
                        rotation: if rotation.is_finite() && rotation.length() > 0.5 {
                            rotation.normalize()
                        } else {
                            Quat::IDENTITY
                        },
                    });
                }
            }
        }
    }

    /// Cells whose square comes within `radius` of `(x, z)`, nearest first.
    pub fn cells_near(&self, x: f32, z: f32, radius: f32) -> Vec<(u32, Vec3)> {
        let mut out: Vec<(f32, u32, Vec3)> = self
            .cells
            .iter()
            .map(|(&h, &c)| (square_distance(c, x, z), h, c))
            .filter(|&(d, _, _)| d <= radius)
            .collect();
        out.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        out.into_iter().map(|(_, h, c)| (h, c)).collect()
    }

    /// A height to start at near `(x, z)`: the centre height of the nearest
    /// cell, or 0.
    pub fn rough_height(&self, x: f32, z: f32) -> f32 {
        self.cells
            .values()
            .min_by(|a, b| square_distance(**a, x, z).total_cmp(&square_distance(**b, x, z)))
            .map_or(0.0, |c| c.y)
    }
}

/// Distance from `(x, z)` to the 400 m square centred on `centre`.
pub fn square_distance(centre: Vec3, x: f32, z: f32) -> f32 {
    let dx = ((centre.x - x).abs() - CELL_SIZE / 2.0).max(0.0);
    let dz = ((centre.z - z).abs() - CELL_SIZE / 2.0).max(0.0);
    (dx * dx + dz * dz).sqrt()
}

/// Terrain geometry in a cell's own space, ready to upload: positions,
/// normals and colour-map coordinates, as a triangle list.
#[derive(Clone, Debug, Default)]
pub struct TerrainMesh {
    /// Where the mesh's origin sits in the world.
    pub origin: Vec3,
    /// Position, normal, texture coordinate (32 bytes per vertex).
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    /// The colour map to texture it with.
    pub colour_map: Option<u32>,
    /// Lowest and highest vertex, relative to `origin`.
    pub bounds: (Vec3, Vec3),
}

impl TerrainMesh {
    pub fn layout() -> VertexLayout {
        VertexLayout::packed(&[
            (VertexSemantic::Position, VertexFormat::Float3),
            (VertexSemantic::Normal, VertexFormat::Float3),
            (VertexSemantic::TexCoord(0), VertexFormat::Float2),
        ])
        .expect("valid layout")
    }

    pub fn vertex_count(&self) -> u32 {
        (self.vertices.len() / 32) as u32
    }

    pub fn vertex_desc(&self) -> VertexBufferDesc {
        VertexBufferDesc {
            layout: Self::layout(),
            vertex_count: self.vertex_count(),
            usage: BufferUsage::Static,
        }
    }

    pub fn index_desc(&self) -> IndexBufferDesc {
        IndexBufferDesc {
            format: IndexFormat::U32,
            index_count: self.indices.len() as u32,
            usage: BufferUsage::Static,
        }
    }

    pub fn index_bytes(&self) -> Vec<u8> {
        self.indices.iter().flat_map(|i| i.to_le_bytes()).collect()
    }

    /// Appends `group`, offset by `offset` from the mesh origin, with
    /// texture coordinates from `uv` of the vertex's mesh-space position.
    fn add_group(&mut self, group: &TerrainGroup<'_>, offset: Vec3, uv: impl Fn(Vec3) -> [f32; 2]) {
        let positions = group.positions();
        let normals = group
            .decl
            .find(DeclUsage::Normal, 0)
            .and_then(|e| group.vertices.read(e))
            .unwrap_or_default();
        let first = self.vertex_count();
        for (i, p) in positions.iter().enumerate() {
            let pos = Vec3::from(*p) + offset;
            let n = normals
                .get(i)
                .and_then(|n| Vec3::new(n[0], n[1], n[2]).try_normalize())
                .unwrap_or(Vec3::Y);
            self.bounds.0 = self.bounds.0.min(pos);
            self.bounds.1 = self.bounds.1.max(pos);
            let t = uv(pos);
            for f in [pos.x, pos.y, pos.z, n.x, n.y, n.z, t[0], t[1]] {
                self.vertices.extend(f.to_le_bytes());
            }
        }
        // Full ranges draw the whole group; fall back to the material ranges.
        let ranges = if group.full_ranges.is_empty() {
            &group.material_ranges
        } else {
            &group.full_ranges
        };
        let mut seen = HashSet::new();
        for r in ranges {
            for tri in group.triangles(r) {
                let mut key = tri;
                key.sort_unstable();
                if tri.iter().all(|&i| usize::from(i) < positions.len()) && seen.insert(key) {
                    self.indices.extend(tri.map(|i| first + u32::from(i)));
                }
            }
        }
    }

    fn empty(origin: Vec3, colour_map: Option<u32>) -> Self {
        Self {
            origin,
            colour_map,
            bounds: (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
            ..Self::default()
        }
    }
}

/// Reads and decodes terrain cell `hash` centred at `centre`.
pub fn terrain_cell<R: Read + Seek>(
    source: &mut AssetSource<R>,
    hash: u32,
    centre: Vec3,
) -> Result<TerrainMesh, AssetError> {
    let data = source.asset(hash, m2_assets::terrain::TERRAIN_TYPE)?;
    let cell = TerrainCell::parse(&data).map_err(|e| AssetError::Terrain {
        hash,
        message: e.to_string(),
    })?;
    let mut mesh = TerrainMesh::empty(centre, cell.colour_map());
    for patch in &cell.patches {
        let offset = Vec3::from(patch.offset);
        for g in &patch.groups {
            mesh.add_group(g, offset, |p| colour_map_uv(p.x, p.z));
        }
    }
    Ok(mesh)
}

/// Reads and decodes low-resolution tile `hash` centred at `centre`,
/// textured with the island-wide map.
pub fn low_res_tile<R: Read + Seek>(
    source: &mut AssetSource<R>,
    hash: u32,
    centre: Vec3,
) -> Result<TerrainMesh, AssetError> {
    let data = source.asset(hash, m2_assets::terrain::LOW_RES_TYPE)?;
    let tile = LowResTile::parse(&data).map_err(|e| AssetError::Terrain {
        hash,
        message: e.to_string(),
    })?;
    let map = tile
        .materials
        .first()
        .and_then(|m| m.textures.first().copied())
        .filter(|&t| t != 0);
    let mut mesh = TerrainMesh::empty(centre, map);
    // The island map follows the cell maps' axes at island scale.
    mesh.add_group(&tile.group, Vec3::ZERO, |p| {
        let w = p + centre;
        [(ISLAND / 2.0 - w.x) / ISLAND, (ISLAND / 2.0 - w.z) / ISLAND]
    });
    Ok(mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_to_a_cell_square() {
        let c = Vec3::new(0.0, 5.0, 0.0);
        assert_eq!(square_distance(c, 0.0, 0.0), 0.0);
        assert_eq!(square_distance(c, 199.0, -150.0), 0.0);
        assert_eq!(square_distance(c, 300.0, 0.0), 100.0);
        assert!((square_distance(c, 230.0, 240.0) - 50.0).abs() < 1e-4);
    }

    #[test]
    fn nearest_cells_first() {
        let mut w = WorldIndex::default();
        w.cells.insert(1, Vec3::new(0.0, 10.0, 0.0));
        w.cells.insert(2, Vec3::new(400.0, 20.0, 0.0));
        w.cells.insert(3, Vec3::new(2000.0, 30.0, 0.0));
        let near: Vec<u32> = w
            .cells_near(250.0, 0.0, 500.0)
            .iter()
            .map(|c| c.0)
            .collect();
        assert_eq!(near, [2, 1]);
        assert_eq!(w.rough_height(1990.0, 0.0), 30.0);
    }
}
