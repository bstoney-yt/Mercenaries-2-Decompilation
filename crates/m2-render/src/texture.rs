//! Textures and materials.
//!
//! A texture is an RGBA8 image with its mip chain, uploaded once and named
//! by a [`TextureHandle`]. A material is the state a draw shades with: an
//! optional base-colour texture, a tint, how it blends, and whether it is
//! lit. Materials are registered under the [`MaterialId`](crate::MaterialId)
//! that sort keys and draw calls already carry, so the queue's grouping by
//! material is also a grouping by GPU state.

use crate::backend::BackendError;
pub use crate::buffer::TextureHandle;

/// Pixel storage of a texture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// 8-bit RGBA, colour data in sRGB (base-colour maps).
    #[default]
    Rgba8Srgb,
    /// 8-bit RGBA, linear data (masks, normal maps).
    Rgba8Unorm,
}

/// Size and format of a texture's top mip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextureDesc {
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
}

impl TextureDesc {
    /// Size of mip `level`; dimensions halve and stop at 1.
    pub fn mip_size(&self, level: u32) -> (u32, u32) {
        ((self.width >> level).max(1), (self.height >> level).max(1))
    }

    /// Mips in a full chain down to 1x1.
    pub fn full_chain_len(&self) -> u32 {
        32 - self.width.max(self.height).max(1).leading_zeros()
    }

    /// Checks that `mips` is a valid chain for this texture: at least the top
    /// mip, no more than a full chain, each exactly `w * h * 4` bytes.
    pub fn validate(&self, mips: &[&[u8]]) -> Result<(), BackendError> {
        if self.width == 0 || self.height == 0 {
            return Err(BackendError::Texture(format!(
                "{}x{} texture",
                self.width, self.height
            )));
        }
        if mips.is_empty() || mips.len() as u32 > self.full_chain_len() {
            return Err(BackendError::Texture(format!(
                "{} mips for a {}x{} texture",
                mips.len(),
                self.width,
                self.height
            )));
        }
        for (level, data) in mips.iter().enumerate() {
            let (w, h) = self.mip_size(level as u32);
            let expected = u64::from(w) * u64::from(h) * 4;
            if data.len() as u64 != expected {
                return Err(BackendError::InitialDataSize {
                    expected,
                    actual: data.len() as u64,
                });
            }
        }
        Ok(())
    }
}

/// Builds the rest of a mip chain below an RGBA8 image with a 2x2 box
/// filter. Returns every level including the top one.
pub fn generate_mips(width: u32, height: u32, rgba: &[u8]) -> Vec<Vec<u8>> {
    let desc = TextureDesc {
        width,
        height,
        format: TextureFormat::Rgba8Srgb,
    };
    let mut levels = vec![rgba.to_vec()];
    for level in 1..desc.full_chain_len() {
        let (pw, ph) = desc.mip_size(level - 1);
        let (w, h) = desc.mip_size(level);
        let prev = &levels[level as usize - 1];
        let mut next = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let sx = (x * 2 + dx).min(pw - 1);
                        let sy = (y * 2 + dy).min(ph - 1);
                        sum += u32::from(prev[((sy * pw + sx) * 4 + c) as usize]);
                    }
                    next[((y * w + x) * 4 + c) as usize] = ((sum + 2) / 4) as u8;
                }
            }
        }
        levels.push(next);
    }
    levels
}

/// How a material's output combines with what is already drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum BlendMode {
    /// Writes colour and depth.
    #[default]
    Opaque,
    /// Opaque, but pixels whose alpha is below `cutoff` are discarded
    /// (foliage, fences).
    Masked { cutoff: f32 },
    /// Alpha blended over the scene, depth-tested but not depth-written.
    /// Draw these back to front (the [`DrawQueue`](crate::DrawQueue) does).
    Translucent,
    /// Added to the scene, scaled by alpha (lights, glows, trails);
    /// depth-tested but not depth-written, drawn with the translucent.
    Additive,
}

impl BlendMode {
    /// Whether it draws over the scene without writing depth, after the
    /// opaque world.
    pub fn is_translucent(self) -> bool {
        matches!(self, Self::Translucent | Self::Additive)
    }
}

/// Everything a draw shades with besides its geometry and transform.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialDesc {
    /// Base colour map; `None` shades with `color` alone.
    pub texture: Option<TextureHandle>,
    /// Multiplied into the base colour (linear RGBA).
    pub color: [f32; 4],
    pub blend: BlendMode,
    /// Draw back faces too.
    pub double_sided: bool,
    /// Shade with the sun and ambient light; unlit materials show their
    /// base colour as is (sky, effects, UI).
    pub lit: bool,
    /// Multiply in the vertex colour, if the vertex layout has one.
    pub vertex_color: bool,
    /// Apply distance fog.
    pub fog: bool,
    /// Clamp texture coordinates to the edge instead of repeating, for
    /// maps that cover one surface exactly (terrain cells), whose filtering
    /// would otherwise pick up the opposite edge.
    pub clamp: bool,
    /// A texture tiled in world space over the base colour near the
    /// camera, for close-up grain (terrain).
    pub detail: Option<DetailMap>,
    /// Fades alpha towards the silhouette, where the surface turns away
    /// from the camera (smoke and other soft volumes).
    pub soft_edge: bool,
    /// Specular highlights: colour, strength map and sharpness. `None` is
    /// a matte surface.
    pub specular: Option<Specular>,
    /// Tangent-space normal map, upload [`TextureFormat::Rgba8Unorm`];
    /// needs vertex tangents (without them the map is ignored).
    pub normal_map: Option<NormalMap>,
    /// Tiled ground layers blended by the vertex colour (`r` weights the
    /// first, `a` the fourth); see [`ground`](Self::ground).
    pub layers: [Option<TerrainLayer>; 4],
    /// Pulls the surface towards the camera in the depth test, so a
    /// surface laid on another (roads on the ground) wins.
    pub depth_bias: bool,
    /// Shade as the game's ground: `texture` is the cell's colour map,
    /// `layers` are weighted by the vertex colour and the colour map makes
    /// up the rest and takes over with distance; `detail` adds signed grain
    /// (upload it [`TextureFormat::Rgba8Unorm`]). Unclamped sun plus
    /// ambient light.
    pub ground: bool,
    /// Linear RGB scale of the sunlight a lit surface takes.
    pub direct: [f32; 3],
    /// Linear RGB scale of the ambient light a lit surface takes.
    pub ambient: [f32; 3],
    /// Light the surface gives off itself (lamps, screens), added after
    /// lighting.
    pub emissive: Option<Emissive>,
    /// Mirror-like reflection of the surroundings (glass, paint, metal).
    pub reflection: Option<Reflection>,
}

/// A reflection of the surroundings, along the view mirrored in the
/// surface, scaled by the specular map when there is one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reflection {
    pub strength: f32,
    /// Schlick's reflectance facing the eye, rising to 1 at grazing
    /// angles; `None` reflects alike at every angle.
    pub fresnel: Option<f32>,
}

/// Light a surface gives off.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Emissive {
    /// Linear RGB scale.
    pub color: [f32; 3],
    /// Per-texel colour, sampled with the base map's coordinates; takes
    /// the specular map's place, so a material has one or the other.
    pub map: Option<TextureHandle>,
}

/// Specular highlights (Blinn-Phong).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Specular {
    /// Linear RGB scale of the highlight.
    pub color: [f32; 3],
    /// Exponent: higher is tighter.
    pub power: f32,
    /// Per-texel highlight colour, sampled with the base map's coordinates.
    pub map: Option<TextureHandle>,
}

/// A tangent-space normal map, sampled with the base map's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NormalMap {
    pub texture: TextureHandle,
    /// Where x and y are stored: `false` in red and green, `true` in alpha
    /// and green (the DXT5 swizzle).
    pub alpha_x: bool,
}

/// One tiled ground layer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TerrainLayer {
    pub texture: TextureHandle,
    /// `u` and `v` as dot products with the world-space `(x, y, z, 1)`.
    pub uv: [[f32; 4]; 2],
}

/// A detail texture: tiled every `tile` metres along world x and z, it
/// scales the base colour by how each texel compares with the texture's
/// average, fading out with distance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetailMap {
    pub texture: TextureHandle,
    pub tile: f32,
}

impl Default for MaterialDesc {
    fn default() -> Self {
        Self {
            texture: None,
            color: [1.0; 4],
            blend: BlendMode::Opaque,
            double_sided: false,
            lit: true,
            vertex_color: false,
            fog: true,
            clamp: false,
            detail: None,
            soft_edge: false,
            specular: None,
            normal_map: None,
            layers: [None; 4],
            depth_bias: false,
            ground: false,
            direct: [1.0; 3],
            ambient: [1.0; 3],
            emissive: None,
            reflection: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chain_lengths() {
        let d = |width, height| TextureDesc {
            width,
            height,
            format: TextureFormat::Rgba8Srgb,
        };
        assert_eq!(d(1, 1).full_chain_len(), 1);
        assert_eq!(d(256, 256).full_chain_len(), 9);
        assert_eq!(d(512, 128).full_chain_len(), 10);
        assert_eq!(d(512, 128).mip_size(8), (2, 1));
        assert_eq!(d(512, 128).mip_size(9), (1, 1));
    }

    #[test]
    fn validates_mip_sizes() {
        let desc = TextureDesc {
            width: 4,
            height: 2,
            format: TextureFormat::Rgba8Srgb,
        };
        let top = [0u8; 32];
        let mid = [0u8; 8];
        let low = [0u8; 4];
        assert!(desc.validate(&[&top]).is_ok());
        assert!(desc.validate(&[&top, &mid, &low]).is_ok());
        assert!(desc.validate(&[]).is_err());
        assert!(desc.validate(&[&top, &mid, &low, &low]).is_err());
        assert!(desc.validate(&[&mid]).is_err());
    }

    #[test]
    fn box_filter_averages() {
        // 2x2: black, white, white, black -> one mid-grey pixel.
        let rgba = [
            0, 0, 0, 255, 255, 255, 255, 255, //
            255, 255, 255, 255, 0, 0, 0, 255,
        ];
        let mips = generate_mips(2, 2, &rgba);
        assert_eq!(mips.len(), 2);
        assert_eq!(mips[1], vec![128, 128, 128, 255]);
    }

    #[test]
    fn box_filter_handles_odd_sizes() {
        let rgba = vec![200u8; 3 * 4];
        let mips = generate_mips(3, 1, &rgba);
        assert_eq!(mips.len(), 2);
        assert_eq!(mips[1], vec![200; 4]);
    }
}
