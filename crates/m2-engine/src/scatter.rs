//! Ground scatter: the grass, shrubs, rubble and litter instanced across
//! each terrain cell, decoded for drawing (see `m2_assets::scatter` for the
//! format).

use std::io::{Read, Seek};

use m2_assets::Scatter;
use m2_assets::model::Material;
use m2_assets::scatter::{SCATTER_TYPE, ScatterMesh};
use m2_assets::vertex::DeclUsage;
use m2_math::glam::{Mat4, Vec3, Vec4};
use m2_render::{BufferUsage, IndexBufferDesc, IndexFormat, VertexBufferDesc};

use crate::assets::{AssetError, AssetSource};
use crate::model::base_texture;
use crate::world::TerrainMesh;

/// One clump mesh, in the terrain vertex layout (position, normal, UV).
#[derive(Clone, Debug)]
pub struct ClumpMesh {
    pub vertices: Vec<u8>,
    pub indices: Vec<u32>,
    pub material: Material,
    /// The mesh's fade-out distances as read (metres, per the format
    /// notes); not yet used, as stopping there left the ground bare.
    pub fade: [f32; 2],
    /// Object to world, one per instance.
    pub instances: Vec<Mat4>,
}

impl ClumpMesh {
    pub fn vertex_desc(&self) -> VertexBufferDesc {
        VertexBufferDesc {
            layout: TerrainMesh::layout(),
            vertex_count: (self.vertices.len() / 32) as u32,
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

    pub fn texture(&self) -> Option<u32> {
        base_texture(&self.material)
    }
}

/// A terrain cell's scatter, every asset stored beside it.
#[derive(Clone, Debug, Default)]
pub struct CellScatter {
    pub meshes: Vec<ClumpMesh>,
}

impl CellScatter {
    /// Bytes, for upload budgets.
    pub fn size(&self) -> usize {
        self.meshes
            .iter()
            .map(|m| m.vertices.len() + m.indices.len() * 4 + m.instances.len() * 64)
            .sum()
    }

    pub fn instance_count(&self) -> usize {
        self.meshes.iter().map(|m| m.instances.len()).sum()
    }

    /// Adds `scatter`'s meshes and instances, for a cell centred at
    /// `centre`.
    pub fn add(&mut self, scatter: &Scatter<'_>, centre: Vec3) {
        let first = self.meshes.len();
        self.meshes.extend(scatter.meshes.iter().map(clump_mesh));
        for (mesh, patch, m) in scatter.placements() {
            let Some(clump) = self.meshes.get_mut(first + usize::from(mesh)) else {
                continue;
            };
            if let Some(world) = instance(m, centre + Vec3::from(patch)) {
                clump.instances.push(world);
            }
        }
        // Meshes nothing uses, or with nothing to draw.
        let mut i = first;
        while i < self.meshes.len() {
            if self.meshes[i].instances.is_empty() || self.meshes[i].indices.is_empty() {
                self.meshes.swap_remove(i);
            } else {
                i += 1;
            }
        }
    }
}

/// Reads the scatter stored with terrain cell `cell`, centred at `centre`.
pub fn cell_scatter<R: Read + Seek>(
    source: &mut AssetSource<R>,
    cell: u32,
    centre: Vec3,
) -> Result<CellScatter, AssetError> {
    let mut out = CellScatter::default();
    for data in source.assets_beside(cell, SCATTER_TYPE)? {
        match Scatter::parse(&data) {
            Ok(s) => out.add(&s, centre),
            Err(e) => eprintln!("scatter in cell {cell:08x}: {e}"),
        }
    }
    Ok(out)
}

fn clump_mesh(m: &ScatterMesh<'_>) -> ClumpMesh {
    let positions = m.positions();
    let normals = m.read(DeclUsage::Normal);
    let uvs = m.read(DeclUsage::TexCoord);
    let mut vertices = Vec::with_capacity(positions.len() * 32);
    for (i, p) in positions.iter().enumerate() {
        let n = normals
            .get(i)
            .and_then(|n| Vec3::new(n[0], n[1], n[2]).try_normalize())
            .unwrap_or(Vec3::Y);
        let uv = uvs.get(i).map_or([0.0; 2], |t| [t[0], t[1]]);
        for f in [p[0], p[1], p[2], n.x, n.y, n.z, uv[0], uv[1]] {
            vertices.extend(f.to_le_bytes());
        }
    }
    ClumpMesh {
        vertices,
        indices: m.triangles().into_iter().flatten().map(u32::from).collect(),
        material: m.material.clone(),
        fade: m.fade,
        instances: Vec::new(),
    }
}

/// An instance's object-to-world matrix from its row-major 3 × 4 matrix
/// (rotation × uniform scale | translation from `origin`). `None` for a
/// degenerate one.
fn instance(m: &[f32; 12], origin: Vec3) -> Option<Mat4> {
    let scale = Vec3::new(m[0], m[4], m[8]).length();
    if !(scale > 0.0 && scale.is_finite()) {
        return None;
    }
    let t = origin + Vec3::new(m[3], m[7], m[11]);
    Some(Mat4::from_cols(
        Vec4::new(m[0], m[4], m[8], 0.0),
        Vec4::new(m[1], m[5], m[9], 0.0),
        Vec4::new(m[2], m[6], m[10], 0.0),
        t.extend(1.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instances_rotate_scale_and_move_from_their_patch() {
        // A quarter turn about y, doubled, 1 m along x from the patch.
        #[rustfmt::skip]
        let m = [
            0.0, 0.0, 2.0, 1.0,
            0.0, 2.0, 0.0, 0.0,
            -2.0, 0.0, 0.0, 0.0,
        ];
        let w = instance(&m, Vec3::new(100.0, 5.0, -50.0)).unwrap();
        assert!(
            w.transform_point3(Vec3::ZERO)
                .abs_diff_eq(Vec3::new(101.0, 5.0, -50.0), 1e-5)
        );
        // Rows act on column vectors: local x goes to -z, z to +x.
        assert!(
            w.transform_vector3(Vec3::X)
                .abs_diff_eq(Vec3::new(0.0, 0.0, -2.0), 1e-5)
        );
        assert!(
            w.transform_vector3(Vec3::Z)
                .abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5)
        );
        assert!(instance(&[0.0; 12], Vec3::ZERO).is_none());
    }
}
