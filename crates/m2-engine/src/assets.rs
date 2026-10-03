//! Reading assets from the player's installation.
//!
//! [`Install`] finds the archives under an install directory, and
//! [`AssetSource`] reads models and textures out of `vz.wad` by name hash,
//! gathering the pieces each asset streams from other blocks (model LODs,
//! texture mip tiers). Decoded blocks are kept in a small LRU cache because
//! neighbouring assets usually share blocks.
//!
//! Nothing here touches the GPU; see [`crate::model`] for uploading.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use m2_assets::model::MODEL_TYPE;
use m2_assets::texture::{BODY, INFO, PixelFormat};
use m2_assets::{
    BlockContents, Model, ModelError, Texture, TextureError, Ucfx, WadArchive, WadError,
};

/// Type hash of texture assets.
pub const TEXTURE_TYPE: u32 = 0xf011_157a;

/// Errors reading assets.
#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0} does not look like a Mercenaries 2 install (no data\\vz.wad)")]
    NotAnInstall(PathBuf),
    #[error(transparent)]
    Wad(#[from] WadError),
    #[error(transparent)]
    Block(#[from] m2_assets::BlockError),
    #[error("model {hash:08x}: {source}")]
    Model { hash: u32, source: ModelError },
    #[error("texture {hash:08x}: {source}")]
    Texture { hash: u32, source: TextureError },
    #[error("terrain {hash:08x}: {message}")]
    Terrain { hash: u32, message: String },
    #[error(transparent)]
    Backend(#[from] m2_render::BackendError),
    #[error("no {kind} with hash {hash:08x}")]
    Missing { kind: &'static str, hash: u32 },
}

/// The files of an installation that the engine reads.
#[derive(Clone, Debug)]
pub struct Install {
    pub root: PathBuf,
    pub data: PathBuf,
}

impl Install {
    /// Checks that `root` (the folder holding `data\`) is an install.
    pub fn find(root: &Path) -> Result<Self, AssetError> {
        let data = root.join("data");
        if data.join("vz.wad").is_file() {
            Ok(Self {
                root: root.to_path_buf(),
                data,
            })
        } else if root.join("vz.wad").is_file() {
            // Pointed at the data folder itself.
            Ok(Self {
                root: root.parent().unwrap_or(root).to_path_buf(),
                data: root.to_path_buf(),
            })
        } else {
            Err(AssetError::NotAnInstall(root.to_path_buf()))
        }
    }

    /// Looks for the game in its usual folders: where the EA app installs
    /// it (seen on a real install) and the older Origin folder (assumed).
    /// `None` when neither holds it.
    pub fn detect() -> Option<Self> {
        Self::detect_in(
            ["ProgramFiles(x86)", "ProgramFiles"]
                .iter()
                .filter_map(std::env::var_os)
                .map(PathBuf::from),
        )
    }

    /// [`detect`](Self::detect), under the given program-files folders.
    pub fn detect_in(program_files: impl IntoIterator<Item = PathBuf>) -> Option<Self> {
        const FOLDERS: [&str; 2] = [
            "EA Games/Mercenaries 2 World in Flames",
            "Origin Games/Mercenaries 2 World in Flames",
        ];
        program_files
            .into_iter()
            .flat_map(|p| FOLDERS.map(|f| p.join(f)))
            .find_map(|root| Self::find(&root).ok())
    }

    pub fn world_archive(&self) -> PathBuf {
        self.data.join("vz.wad")
    }

    /// Opens `vz.wad`, the archive holding the island and everything on it.
    pub fn open_world(&self) -> Result<AssetSource<BufReader<File>>, AssetError> {
        let path = self.world_archive();
        let file = File::open(&path).map_err(|source| AssetError::Io { path, source })?;
        AssetSource::new(BufReader::new(file))
    }
}

/// A model asset with every streamed chunk, owned. Parse it with
/// [`ModelData::parse`].
#[derive(Clone, Debug)]
pub struct ModelData {
    pub hash: u32,
    pub base: Vec<u8>,
    /// Streamed geometry chunks in slot order.
    pub chunks: Vec<Vec<u8>>,
}

impl ModelData {
    pub fn parse(&self) -> Result<Model<'_>, AssetError> {
        let err = |source| AssetError::Model {
            hash: self.hash,
            source,
        };
        let mut model = Model::parse(&self.base).map_err(err)?;
        for c in &self.chunks {
            model.add_streamed(c).map_err(err)?;
        }
        Ok(model)
    }
}

/// A texture decoded to RGBA8 with its mip chain.
#[derive(Clone, Debug)]
pub struct DecodedTexture {
    pub width: u32,
    pub height: u32,
    /// Top mip first, each half the size of the one before.
    pub mips: Vec<Vec<u8>>,
    /// The stored format has an alpha channel.
    pub alpha: bool,
}

/// The tier (`_Q<n>`) a streamed block belongs to.
fn block_tier(path: &str) -> Option<usize> {
    let at = path.rfind("_Q")?;
    Some(path[at + 2..].chars().next()?.to_digit(10)? as usize)
}

/// Reads assets from one archive, caching decoded blocks.
#[derive(Debug)]
pub struct AssetSource<R> {
    wad: WadArchive<R>,
    cache: HashMap<usize, (u64, Arc<Vec<u8>>)>,
    cached_bytes: usize,
    budget: usize,
    clock: u64,
}

impl<R: Read + Seek> AssetSource<R> {
    pub fn new(reader: R) -> Result<Self, AssetError> {
        Ok(Self {
            wad: WadArchive::open(reader)?,
            cache: HashMap::new(),
            cached_bytes: 0,
            budget: 256 << 20,
            clock: 0,
        })
    }

    pub fn archive(&self) -> &WadArchive<R> {
        &self.wad
    }

    /// Caps the decoded-block cache (default 256 MiB).
    pub fn set_cache_budget(&mut self, bytes: usize) {
        self.budget = bytes;
    }

    /// A decoded block, from the cache if it is there.
    pub fn block(&mut self, block: usize) -> Result<Arc<Vec<u8>>, AssetError> {
        self.clock += 1;
        if let Some((used, data)) = self.cache.get_mut(&block) {
            *used = self.clock;
            return Ok(data.clone());
        }
        let data = Arc::new(self.wad.read_block(block)?);
        self.cached_bytes += data.len();
        self.cache.insert(block, (self.clock, data.clone()));
        while self.cached_bytes > self.budget && self.cache.len() > 1 {
            let (&oldest, _) = self
                .cache
                .iter()
                .min_by_key(|(_, (used, _))| *used)
                .expect("cache is not empty");
            if let Some((_, evicted)) = self.cache.remove(&oldest) {
                self.cached_bytes -= evicted.len();
            }
        }
        Ok(data)
    }

    /// Every block holding a piece of asset `hash`.
    fn blocks_of(&self, hash: u32) -> Vec<usize> {
        let mut blocks: Vec<usize> = self
            .wad
            .index()
            .assets_by_hash(hash)
            .flat_map(|a| std::iter::once(a.block).chain(a.alt_blocks()))
            .map(usize::from)
            .collect();
        blocks.sort_unstable();
        blocks.dedup();
        blocks
    }

    /// The (unstreamed) asset `hash` of type `type_hash`.
    pub fn asset(&mut self, hash: u32, type_hash: u32) -> Result<Vec<u8>, AssetError> {
        for b in self.blocks_of(hash) {
            let decoded = self.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash == type_hash && e.name_hash == hash && e.unk_0x08 == 0 {
                    return Ok(data.to_vec());
                }
            }
        }
        Err(AssetError::Missing {
            kind: "asset",
            hash,
        })
    }

    /// Every asset of type `type_hash` stored in the same blocks as asset
    /// `hash` (a terrain cell's scatter, say), each once.
    pub fn assets_beside(&mut self, hash: u32, type_hash: u32) -> Result<Vec<Vec<u8>>, AssetError> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for b in self.blocks_of(hash) {
            let decoded = self.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash == type_hash && e.unk_0x08 == 0 && seen.insert(e.name_hash) {
                    out.push(data.to_vec());
                }
            }
        }
        Ok(out)
    }

    /// Model `hash` with all its streamed geometry.
    pub fn model(&mut self, hash: u32) -> Result<ModelData, AssetError> {
        let (mut base, mut chunks) = (None, Vec::new());
        for b in self.blocks_of(hash) {
            let decoded = self.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash == MODEL_TYPE && e.name_hash == hash {
                    if e.unk_0x08 == 0 {
                        base = Some(data.to_vec());
                    } else {
                        chunks.push((e.unk_0x08, data.to_vec()));
                    }
                }
            }
        }
        chunks.sort_by_key(|c| c.0);
        let base = base.ok_or(AssetError::Missing {
            kind: "model",
            hash,
        })?;
        Ok(ModelData {
            hash,
            base,
            chunks: chunks.into_iter().map(|c| c.1).collect(),
        })
    }

    /// Texture `hash` decoded to RGBA8, starting at the largest mip no wider
    /// than `max_width` and going down to 1x1. Uses the streamed tiers when
    /// they are present and the resident mips otherwise.
    pub fn texture(&mut self, hash: u32, max_width: u32) -> Result<DecodedTexture, AssetError> {
        let mut base = None;
        let mut tiers: [Option<Vec<u8>>; 3] = Default::default();
        for b in self.blocks_of(hash) {
            let tier = block_tier(&self.wad.index().blocks()[b].path);
            let decoded = self.block(b)?;
            for (e, data) in BlockContents::parse(&decoded)?.iter() {
                if e.type_hash != TEXTURE_TYPE || e.name_hash != hash {
                    continue;
                }
                let Ok(u) = Ucfx::parse(data) else { continue };
                if u.top_level(INFO).is_some() {
                    base = Some(data.to_vec());
                } else if let (Some(t), Some(body)) = (tier.filter(|&t| t < 3), u.top_level(BODY)) {
                    tiers[t] = Some(body.to_vec());
                }
            }
        }
        let base = base.ok_or(AssetError::Missing {
            kind: "texture",
            hash,
        })?;
        let err = |source| AssetError::Texture { hash, source };
        let tex = Texture::parse(&base).map_err(err)?;
        let format = tex.info.format;
        let has_alpha = matches!(
            format,
            PixelFormat::Dxt3 | PixelFormat::Dxt5 | PixelFormat::A8R8G8B8
        );
        let chain = tex.assemble([
            tiers[0].as_deref(),
            tiers[1].as_deref(),
            tiers[2].as_deref(),
        ]);
        let all = match &chain {
            Ok(c) => tex.chain_mips(c).map_err(err)?,
            Err(_) => tex.resident_mips(),
        };
        let mut mips = Vec::new();
        let mut size: Option<(u32, u32)> = None;
        for m in all.iter().skip_while(|m| m.width > max_width.max(1)) {
            let expected = match size {
                None => (m.width, m.height),
                Some((w, h)) => ((w / 2).max(1), (h / 2).max(1)),
            };
            // Stop at a gap in the chain rather than upload a broken one.
            if (m.width, m.height) != expected {
                break;
            }
            mips.push(m.to_rgba8(format).map_err(err)?);
            size = Some(expected);
        }
        let (width, height) = all
            .iter()
            .find(|m| m.width <= max_width.max(1))
            .map(|m| (m.width, m.height))
            .ok_or(AssetError::Missing {
                kind: "texture mip",
                hash,
            })?;
        let alpha = has_alpha && mips.first().is_some_and(|m| looks_cut_out(m));
        Ok(DecodedTexture {
            width,
            height,
            mips,
            alpha,
        })
    }
}

/// Whether an RGBA8 image's alpha cuts shapes out (leaves, fences: some
/// pixels clear, most fully clear or fully solid) rather than holding a
/// mask for something else (shine, dirt), which is mostly in between.
pub fn looks_cut_out(rgba: &[u8]) -> bool {
    let n = rgba.len() / 4;
    if n == 0 {
        return false;
    }
    let (mut clear, mut between) = (0usize, 0usize);
    for p in rgba.chunks_exact(4) {
        if p[3] < 128 {
            clear += 1;
        }
        if (32..224).contains(&p[3]) {
            between += 1;
        }
    }
    clear * 100 >= n && between * 100 <= n * 35
}

#[cfg(test)]
mod tests {
    #[test]
    fn cut_outs_are_mostly_clear_or_solid() {
        let image =
            |alphas: &[u8]| -> Vec<u8> { alphas.iter().flat_map(|&a| [9, 9, 9, a]).collect() };
        assert!(super::looks_cut_out(&image(&[0, 0, 255, 255, 255, 128])));
        // A shine mask: all in between.
        assert!(!super::looks_cut_out(&image(&[60, 90, 120, 150, 200, 100])));
        // Solid everywhere.
        assert!(!super::looks_cut_out(&image(&[255; 8])));
    }

    use super::*;

    #[test]
    fn tiers_from_block_paths() {
        assert_eq!(block_tier(r"vz\textures_P0_Q1"), Some(1));
        assert_eq!(block_tier(r"vz\textures_P0"), None);
        assert_eq!(block_tier("x_Qz"), None);
    }

    #[test]
    fn install_needs_the_world_archive() {
        let dir = std::env::temp_dir().join(format!("m2-install-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("data")).unwrap();
        assert!(matches!(
            Install::find(&dir),
            Err(AssetError::NotAnInstall(_))
        ));
        std::fs::write(dir.join("data").join("vz.wad"), b"").unwrap();
        let install = Install::find(&dir).unwrap();
        assert_eq!(install.data, dir.join("data"));
        let via_data = Install::find(&dir.join("data")).unwrap();
        assert_eq!(via_data.root, dir);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn installs_are_found_in_the_usual_folders() {
        let pf = std::env::temp_dir().join(format!("m2-detect-{}", std::process::id()));
        let root = pf.join("EA Games/Mercenaries 2 World in Flames");
        std::fs::create_dir_all(root.join("data")).unwrap();
        assert!(Install::detect_in([pf.clone()]).is_none());
        std::fs::write(root.join("data").join("vz.wad"), b"").unwrap();
        assert_eq!(Install::detect_in([pf.clone()]).unwrap().root, root);
        std::fs::remove_dir_all(&pf).unwrap();
    }
}
