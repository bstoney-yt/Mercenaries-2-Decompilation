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
        let r = HumanRagdoll::from_physics(&Physics::parse(phy2)?)?;
        println!(
            "{} bones, {:.1} kg, {} joints",
            r.bones.len(),
            r.mass(),
            r.joints.len()
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
