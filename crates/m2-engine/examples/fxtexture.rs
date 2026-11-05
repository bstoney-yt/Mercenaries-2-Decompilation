//! Writes the atlas cell each emitter of the effects given draws with,
//! as PPM images into a folder of yours, and prints the cells' sizes: a
//! look at what an effect's particles are pictures of.
//!
//! ```text
//! cargo run --release -p m2-engine --example fxtexture -- "<install dir>" <out dir> <effect hex>...
//! ```
//!
//! The images are of your own install's data: keep them local.

use std::io::Write;
use std::path::Path;

use m2_assets::effect::{ATLAS_DICT, ATLAS_TEXTURE, ATLAS_TYPE, Atlas, EFFECT_TYPE};
use m2_engine::Install;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [root, out, effects @ ..] = &args[..] else {
        return Err("usage: fxtexture <install dir> <out dir> <effect hex>...".into());
    };
    let install = Install::find(Path::new(root))?;
    let mut source = install.open_world()?;
    let atlas = Atlas::parse(&source.asset(ATLAS_DICT, ATLAS_TYPE)?)?;
    let tex = source.texture(ATLAS_TEXTURE, u32::MAX)?;
    let (w, h) = (tex.width as usize, tex.height as usize);
    let px = &tex.mips[0];
    println!("atlas {w}x{h}, {} bytes", px.len());
    for e in effects {
        let hash = u32::from_str_radix(e.trim_start_matches("0x"), 16)?;
        let fx = m2_assets::effect::parse(&source.asset(hash, EFFECT_TYPE)?)?;
        for (k, em) in fx.emitters.iter().enumerate() {
            for t in &em.textures {
                let Some(&[l, b, cw, ch]) = atlas.rects.get(t) else {
                    println!("{hash:08x}#{k}: texture {t:08x} not on the atlas");
                    continue;
                };
                let x0 = (l * w as f32) as usize;
                let y1 = h - (b * h as f32) as usize;
                let (pw, ph) = ((cw * w as f32) as usize, (ch * h as f32) as usize);
                let y0 = y1.saturating_sub(ph);
                println!("{hash:08x}#{k}: texture {t:08x} {pw}x{ph} at {x0},{y0}");
                let path = Path::new(out).join(format!("{hash:08x}_{k}_{t:08x}.ppm"));
                let mut f = std::fs::File::create(&path)?;
                write!(f, "P6\n{pw} {ph}\n255\n")?;
                for y in y0..y0 + ph {
                    for x in x0..x0 + pw {
                        let i = (y * w + x) * 4;
                        let a = px.get(i + 3).copied().unwrap_or(255) as u16;
                        // Over black, by its alpha.
                        let c = |v: u8| ((v as u16 * a) / 255) as u8;
                        f.write_all(&[c(px[i]), c(px[i + 1]), c(px[i + 2])])?;
                    }
                }
            }
        }
    }
    Ok(())
}
