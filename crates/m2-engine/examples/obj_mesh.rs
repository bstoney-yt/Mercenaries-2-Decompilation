//! Turns a Wavefront `.obj` into a Rust module of constants, so a model
//! made outside the game can be built into the engine as code: its
//! vertices, its polygons and, per object, its name, material and
//! polygons. Normals and texture coordinates are not read (flat-shaded
//! models are lit from their faces).
//!
//! ```text
//! cargo run -p m2-engine --example obj_mesh -- <model.obj> > <module.rs>
//! ```
//!
//! Used for the Osprey's body (`m2_engine::osprey_mesh`).

use std::error::Error;
use std::fmt::Write as _;

type Res<T> = Result<T, Box<dyn Error>>;

fn main() -> Res<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: obj_mesh <model.obj>")?;
    let text = std::fs::read_to_string(&path)?;
    let mut vertices: Vec<[f32; 3]> = Vec::new();
    let mut sizes: Vec<u8> = Vec::new();
    let mut indices: Vec<u16> = Vec::new();
    // (object, material, first polygon, polygons)
    let mut parts: Vec<(String, String, usize, usize)> = Vec::new();
    let (mut object, mut material) = (String::from("default"), String::from("default"));
    for line in text.lines() {
        let mut words = line.split_whitespace();
        match words.next() {
            Some("v") => {
                let v: Vec<f32> = words.take(3).map(str::parse).collect::<Result<_, _>>()?;
                if v.len() != 3 {
                    return Err(format!("short vertex: {line}").into());
                }
                vertices.push([v[0], v[1], v[2]]);
            }
            Some("o") | Some("g") => object = words.collect::<Vec<_>>().join(" "),
            Some("usemtl") => material = words.collect::<Vec<_>>().join(" "),
            Some("f") => {
                let corners: Vec<u16> = words
                    .map(|w| -> Res<u16> {
                        let i: i64 = w.split('/').next().unwrap_or("").parse()?;
                        let i = if i < 0 {
                            vertices.len() as i64 + i
                        } else {
                            i - 1
                        };
                        Ok(u16::try_from(i)?)
                    })
                    .collect::<Res<_>>()?;
                if corners.len() < 3 {
                    return Err(format!("short face: {line}").into());
                }
                let same = parts
                    .last()
                    .is_some_and(|p| p.0 == object && p.1 == material && p.2 + p.3 == sizes.len());
                if same {
                    parts.last_mut().unwrap().3 += 1;
                } else {
                    parts.push((object.clone(), material.clone(), sizes.len(), 1));
                }
                sizes.push(u8::try_from(corners.len())?);
                indices.extend(corners);
            }
            _ => {}
        }
    }
    let name = std::path::Path::new(&path)
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let mut out = String::new();
    writeln!(
        out,
        "//! Generated from `{name}` by the `obj_mesh` example; do not edit."
    )?;
    writeln!(
        out,
        "//! {} vertices, {} polygons, {} parts.",
        vertices.len(),
        sizes.len(),
        parts.len()
    )?;
    writeln!(out)?;
    writeln!(
        out,
        "/// One object of the model: its name, material and polygons (a range of"
    )?;
    writeln!(out, "/// [`SIZES`]).")?;
    writeln!(out, "#[derive(Debug)]")?;
    writeln!(out, "pub struct Part {{")?;
    writeln!(out, "    pub name: &'static str,")?;
    writeln!(out, "    pub material: &'static str,")?;
    writeln!(out, "    pub polygons: std::ops::Range<usize>,")?;
    writeln!(out, "}}")?;
    writeln!(out)?;
    writeln!(out, "#[rustfmt::skip]")?;
    writeln!(out, "pub const PARTS: [Part; {}] = [", parts.len())?;
    for (o, m, first, n) in &parts {
        writeln!(
            out,
            "    Part {{ name: {o:?}, material: {m:?}, polygons: {first}..{} }},",
            first + n
        )?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// The model's vertices, as the file gives them (metres, z up)."
    )?;
    writeln!(out, "#[rustfmt::skip]")?;
    writeln!(
        out,
        "pub const VERTICES: [[f32; 3]; {}] = [",
        vertices.len()
    )?;
    for v in &vertices {
        writeln!(out, "    [{:?}, {:?}, {:?}],", v[0], v[1], v[2])?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(out, "/// How many corners each polygon has, in order.")?;
    writeln!(out, "#[rustfmt::skip]")?;
    writeln!(out, "pub const SIZES: [u8; {}] = [", sizes.len())?;
    for chunk in sizes.chunks(32) {
        let line: Vec<String> = chunk.iter().map(u8::to_string).collect();
        writeln!(out, "    {},", line.join(", "))?;
    }
    writeln!(out, "];")?;
    writeln!(out)?;
    writeln!(
        out,
        "/// Each polygon's corners in turn (indices into [`VERTICES`]), wound"
    )?;
    writeln!(
        out,
        "/// anticlockwise seen from outside, as `.obj` files are."
    )?;
    writeln!(out, "#[rustfmt::skip]")?;
    writeln!(out, "pub const CORNERS: [u16; {}] = [", indices.len())?;
    for chunk in indices.chunks(16) {
        let line: Vec<String> = chunk.iter().map(u16::to_string).collect();
        writeln!(out, "    {},", line.join(", "))?;
    }
    writeln!(out, "];")?;
    print!("{out}");
    Ok(())
}
