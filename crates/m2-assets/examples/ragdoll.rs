//! Prints the human ragdoll of your own installation: the model asset
//! `b9bac8b5` in `vz.wad`'s `resident` block that the 360 loads for every
//! person's ragdoll (0x8218F980), with its nodes and its `PHY2` parts,
//! bodies (mass) and joints.
//!
//! ```text
//! cargo run -p m2-assets --example ragdoll -- <vz.wad>
//! ```
//!
//! Prints names, hashes and numbers only.

use std::fs::File;
use std::io::BufReader;

use m2_assets::ragdoll::HumanRagdoll;
use m2_assets::{BlockContents, Model, Physics, Ucfx, WadArchive, name_hash};

/// The bone's name, where one of the `Bone_` names hashes to it.
fn name(hash: u32) -> &'static str {
    const NAMES: [&str; 12] = [
        "Hips", "Spine2", "Chest", "Head", "LBicep", "LForearm", "RBicep", "RForearm", "LThigh",
        "LShin", "RThigh", "RShin",
    ];
    NAMES
        .iter()
        .find(|n| name_hash(&format!("Bone_{n}")) == hash)
        .copied()
        .unwrap_or("?")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("usage: ragdoll <vz.wad>")?;
    let mut wad = WadArchive::open(BufReader::new(File::open(path)?))?;
    let block = wad
        .index()
        .find_block(r"blocks\VZ\resident_P000_Q3.block")
        .ok_or("no resident block")?;
    let decoded = wad.read_block(block)?;
    for (e, data) in BlockContents::parse(&decoded)?.iter() {
        if e.name_hash != m2_assets::ragdoll::RAGDOLL || e.type_hash != m2_assets::model::MODEL_TYPE
        {
            continue;
        }
        let u = Ucfx::parse(data)?;
        let model = Model::parse(data)?;
        println!("{} nodes", model.nodes.len());
        let phy2 = u.top_level(*b"PHY2").ok_or("no PHY2")?;
        let r = HumanRagdoll::from_model(&model, &Physics::parse(phy2)?)?;
        println!(
            "{} bones, {:.1} kg, {} joints",
            r.bones.len(),
            r.mass(),
            r.joints.len()
        );
        // The check line: the game's one human ragdoll, by its asset's
        // name hash, as the 360 reads it (0x8218F980): 12 bodies, 11
        // joints.
        let ok = e.name_hash == 0xb9ba_c8b5 && r.bones.len() == 12 && r.joints.len() == 11;
        println!(
            "ragdoll-check: asset {:08x}: {} bones, {} joints: {}",
            e.name_hash,
            r.bones.len(),
            r.joints.len(),
            if ok { "PASS" } else { "FAIL" }
        );
        for (i, b) in r.bones.iter().enumerate() {
            println!(
                "bone {i:2} node {:2} {:08x} {:<13} parent {:?} {:.3} kg {:?}",
                b.node,
                b.node_hash,
                name(b.node_hash),
                b.parent,
                b.mass,
                b.shape
            );
        }
        for b in &r.bones {
            let m = m2_math::glam::Mat4::from_cols_array(&b.bind);
            let (_, q, t) = m.to_scale_rotation_translation();
            println!(
                "bind {:<9} at {:.3?} x {:.2?} y {:.2?} shape at {:.3?}",
                name(b.node_hash),
                t.to_array(),
                (q * m2_math::glam::Vec3::X).to_array(),
                (q * m2_math::glam::Vec3::Y).to_array(),
                b.shape_place
            );
        }
        for (i, j) in r.joints.iter().enumerate() {
            println!(
                "joint {i:2} {} to {} {:?}",
                name(r.bones[j.bone].node_hash),
                name(r.bones[j.other].node_hash),
                j.limits
            );
        }
    }
    Ok(())
}
