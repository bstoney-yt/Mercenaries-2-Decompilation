//! Radios: the entities and templates with a `MusicSource` record, each
//! playing a playlist the scripts fill (`MrxMusic.AddMusicPlaylist` and
//! `BindPlaylistCue`, through `Sound.AddMusicSourcePlaylist` and
//! `AddCueToMusicSourcePlaylist`) to whoever comes near.
//!
//! # Provenance
//!
//! From samples: the record's two fields, a name hash and a float. The
//! first is the playlist's name hash ([`m2_assets::name_hash`]): the five
//! outposts' radios name `mu_src_al_op_01`, `mu_src_ch_op_01`,
//! `mu_src_up_op_01`, `mu_src_plav_op_01` and `mu_src_pr_hq_01`, all
//! playlists the scripts make. The second is read as the radio's reach in
//! metres (25 for most, 50 for the pirates', 125 for one bar): inferred
//! from the values, not checked against the engine. The field names
//! themselves aren't known.

use std::collections::HashMap;

use m2_assets::Layer;
use m2_math::glam::Vec3;

/// The playlist field's name hash.
const PLAYLIST_FIELD: u32 = 0x37ed_c753;
/// The reach field's name hash.
const REACH_FIELD: u32 = 0xe068_6a68;

/// A radio placed in the world.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MusicSource {
    /// The entity, and the layer that placed it (it plays while that
    /// layer is in).
    pub entity: u32,
    pub layer: u32,
    pub at: Vec3,
    /// The playlist's name hash.
    pub playlist: u32,
    /// How near (metres) it is heard.
    pub reach: f32,
}

/// The `MusicSource` records of a layer or the template database: by
/// entity (or template) id, the playlist and the reach.
pub fn read(db: &Layer<'_>) -> HashMap<u32, (u32, f32)> {
    let Some(c) = db.component("MusicSource") else {
        return HashMap::new();
    };
    let offset = |hash: u32| {
        c.fields
            .iter()
            .find(|f| f.name_hash == hash)
            .map(|f| usize::from(f.offset))
    };
    let (Some(list), Some(reach)) = (offset(PLAYLIST_FIELD), offset(REACH_FIELD)) else {
        return HashMap::new();
    };
    c.records()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(id, r)| {
            let word = |o: usize| {
                r.get(o..o + 4)
                    .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            };
            let playlist = word(list)?;
            let reach = f32::from_bits(word(reach)?);
            (playlist != 0 && reach.is_finite() && reach > 0.0).then_some((id, (playlist, reach)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_outposts_playlists_are_their_name_hashes() {
        // The values the radios carry, from samples.
        assert_eq!(m2_assets::name_hash("mu_src_al_op_01"), 0x529d_0b1c);
        assert_eq!(m2_assets::name_hash("mu_src_ch_op_01"), 0x7ea9_007a);
        assert_eq!(m2_assets::name_hash("mu_src_up_op_01"), 0x74bc_aae4);
        assert_eq!(m2_assets::name_hash("mu_src_plav_op_01"), 0xb137_3d26);
        assert_eq!(m2_assets::name_hash("mu_src_pr_hq_01"), 0xa01c_a9e1);
        assert_eq!(f32::from_bits(0x41c8_0000), 25.0);
    }
}
