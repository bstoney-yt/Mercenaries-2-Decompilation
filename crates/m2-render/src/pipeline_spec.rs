//! A render pipeline described by what makes it, independent of any one
//! run's numbering, so the pipelines a run needed can be written down and
//! built ahead of the draws of the next ([`RenderBackend::prepare_pipeline`]).
//!
//! The text form ([`PipelineSpec::to_line`], [`write_specs`]) is the
//! port's own: one pipeline a line, every field spelled out, so a file
//! from another build is read or skipped line by line, never misread.
//!
//! Provenance: port-side and invented. The 360 ships its shaders
//! precompiled (`shaders.bin`) and builds nothing at draw time; this only
//! moves when the port's pipelines get built, not what they draw.
//!
//! [`RenderBackend::prepare_pipeline`]: crate::RenderBackend::prepare_pipeline

use crate::backend::Topology;
use crate::buffer::{IndexFormat, VertexElement, VertexFormat, VertexLayout, VertexSemantic};

/// How a pipeline blends: as the material's [`BlendMode`](crate::texture::BlendMode),
/// without the cut-out's threshold (which the shader reads, not the pipeline).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PipelineBlend {
    Opaque,
    Masked,
    Translucent,
    Additive,
}

/// One render pipeline: a draw's (with its glow layer's twin, where the
/// blend has one) or a shadow caster's.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PipelineSpec {
    Draw {
        layout: VertexLayout,
        topology: Topology,
        strip_index: Option<IndexFormat>,
        blend: PipelineBlend,
        double_sided: bool,
        depth_bias: bool,
    },
    Shadow {
        layout: VertexLayout,
        topology: Topology,
        strip_index: Option<IndexFormat>,
        /// Alpha-tested: runs a fragment stage to cut out.
        masked: bool,
    },
}

/// The first line of a file of specs ([`write_specs`]).
pub const HEADER: &str = "m2 pipelines 1";

fn topology_name(t: Topology) -> &'static str {
    match t {
        Topology::PointList => "points",
        Topology::LineList => "lines",
        Topology::LineStrip => "linestrip",
        Topology::TriangleList => "triangles",
        Topology::TriangleStrip => "tristrip",
        Topology::TriangleFan => "trifan",
    }
}

fn topology_named(s: &str) -> Option<Topology> {
    Some(match s {
        "points" => Topology::PointList,
        "lines" => Topology::LineList,
        "linestrip" => Topology::LineStrip,
        "triangles" => Topology::TriangleList,
        "tristrip" => Topology::TriangleStrip,
        "trifan" => Topology::TriangleFan,
        _ => return None,
    })
}

fn strip_name(s: Option<IndexFormat>) -> &'static str {
    match s {
        None => "-",
        Some(IndexFormat::U16) => "u16",
        Some(IndexFormat::U32) => "u32",
    }
}

fn strip_named(s: &str) -> Option<Option<IndexFormat>> {
    Some(match s {
        "-" => None,
        "u16" => Some(IndexFormat::U16),
        "u32" => Some(IndexFormat::U32),
        _ => return None,
    })
}

fn blend_name(b: PipelineBlend) -> &'static str {
    match b {
        PipelineBlend::Opaque => "opaque",
        PipelineBlend::Masked => "masked",
        PipelineBlend::Translucent => "translucent",
        PipelineBlend::Additive => "additive",
    }
}

fn blend_named(s: &str) -> Option<PipelineBlend> {
    Some(match s {
        "opaque" => PipelineBlend::Opaque,
        "masked" => PipelineBlend::Masked,
        "translucent" => PipelineBlend::Translucent,
        "additive" => PipelineBlend::Additive,
        _ => return None,
    })
}

fn flag(b: bool) -> &'static str {
    if b { "1" } else { "0" }
}

fn flagged(s: &str) -> Option<bool> {
    match s {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

fn semantic_name(s: VertexSemantic) -> String {
    match s {
        VertexSemantic::Position => "pos".into(),
        VertexSemantic::Normal => "nrm".into(),
        VertexSemantic::Tangent => "tan".into(),
        VertexSemantic::Binormal => "bin".into(),
        VertexSemantic::Color => "col".into(),
        VertexSemantic::TexCoord(n) => format!("tc{n}"),
        VertexSemantic::BlendWeight => "bw".into(),
        VertexSemantic::BlendIndices => "bi".into(),
    }
}

fn semantic_named(s: &str) -> Option<VertexSemantic> {
    Some(match s {
        "pos" => VertexSemantic::Position,
        "nrm" => VertexSemantic::Normal,
        "tan" => VertexSemantic::Tangent,
        "bin" => VertexSemantic::Binormal,
        "col" => VertexSemantic::Color,
        "bw" => VertexSemantic::BlendWeight,
        "bi" => VertexSemantic::BlendIndices,
        _ => VertexSemantic::TexCoord(s.strip_prefix("tc")?.parse().ok()?),
    })
}

fn format_name(f: VertexFormat) -> &'static str {
    match f {
        VertexFormat::Float1 => "f1",
        VertexFormat::Float2 => "f2",
        VertexFormat::Float3 => "f3",
        VertexFormat::Float4 => "f4",
        VertexFormat::UNorm4x8 => "un4",
        VertexFormat::Bgra8 => "bgra",
        VertexFormat::UInt4x8 => "u4",
        VertexFormat::Half2 => "h2",
        VertexFormat::Half4 => "h4",
    }
}

fn format_named(s: &str) -> Option<VertexFormat> {
    Some(match s {
        "f1" => VertexFormat::Float1,
        "f2" => VertexFormat::Float2,
        "f3" => VertexFormat::Float3,
        "f4" => VertexFormat::Float4,
        "un4" => VertexFormat::UNorm4x8,
        "bgra" => VertexFormat::Bgra8,
        "u4" => VertexFormat::UInt4x8,
        "h2" => VertexFormat::Half2,
        "h4" => VertexFormat::Half4,
        _ => return None,
    })
}

/// `stride` then each element as `semantic:format:offset`, comma-separated.
fn layout_text(layout: &VertexLayout) -> String {
    let elements: Vec<String> = layout
        .elements()
        .iter()
        .map(|e| {
            format!(
                "{}:{}:{}",
                semantic_name(e.semantic),
                format_name(e.format),
                e.offset
            )
        })
        .collect();
    format!("{} {}", layout.stride(), elements.join(","))
}

fn layout_from(stride: &str, elements: &str) -> Option<VertexLayout> {
    let stride = stride.parse().ok()?;
    let elements = elements
        .split(',')
        .map(|e| {
            let mut parts = e.split(':');
            let semantic = semantic_named(parts.next()?)?;
            let format = format_named(parts.next()?)?;
            let offset = parts.next()?.parse().ok()?;
            parts.next().is_none().then_some(VertexElement {
                semantic,
                format,
                offset,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    VertexLayout::new(elements, stride).ok()
}

impl PipelineSpec {
    /// The vertex layout it reads.
    pub fn layout(&self) -> &VertexLayout {
        match self {
            Self::Draw { layout, .. } | Self::Shadow { layout, .. } => layout,
        }
    }

    /// One line: `draw <topology> <strip> <blend> <double-sided> <depth
    /// bias> <stride> <elements>` or `shadow <topology> <strip> <masked>
    /// <stride> <elements>`.
    pub fn to_line(&self) -> String {
        match self {
            Self::Draw {
                layout,
                topology,
                strip_index,
                blend,
                double_sided,
                depth_bias,
            } => format!(
                "draw {} {} {} {} {} {}",
                topology_name(*topology),
                strip_name(*strip_index),
                blend_name(*blend),
                flag(*double_sided),
                flag(*depth_bias),
                layout_text(layout)
            ),
            Self::Shadow {
                layout,
                topology,
                strip_index,
                masked,
            } => format!(
                "shadow {} {} {} {}",
                topology_name(*topology),
                strip_name(*strip_index),
                flag(*masked),
                layout_text(layout)
            ),
        }
    }

    /// The spec a [`to_line`](Self::to_line) line gives, or none for a
    /// line it can't have written.
    pub fn parse(line: &str) -> Option<Self> {
        let words: Vec<&str> = line.split_whitespace().collect();
        match words.as_slice() {
            [
                "draw",
                topology,
                strip,
                blend,
                sided,
                bias,
                stride,
                elements,
            ] => Some(Self::Draw {
                topology: topology_named(topology)?,
                strip_index: strip_named(strip)?,
                blend: blend_named(blend)?,
                double_sided: flagged(sided)?,
                depth_bias: flagged(bias)?,
                layout: layout_from(stride, elements)?,
            }),
            ["shadow", topology, strip, masked, stride, elements] => Some(Self::Shadow {
                topology: topology_named(topology)?,
                strip_index: strip_named(strip)?,
                masked: flagged(masked)?,
                layout: layout_from(stride, elements)?,
            }),
            _ => None,
        }
    }
}

/// The file's text for `specs`: [`HEADER`], then one line each, sorted and
/// each once, so the same set always writes the same file.
pub fn write_specs<'a>(specs: impl IntoIterator<Item = &'a PipelineSpec>) -> String {
    let mut lines: Vec<String> = specs.into_iter().map(PipelineSpec::to_line).collect();
    lines.sort();
    lines.dedup();
    let mut text = String::from(HEADER);
    text.push('\n');
    for line in lines {
        text.push_str(&line);
        text.push('\n');
    }
    text
}

/// The specs in a [`write_specs`] file, and how many lines were skipped
/// as unreadable; none at all for another header.
pub fn read_specs(text: &str) -> (Vec<PipelineSpec>, usize) {
    let mut lines = text.lines();
    if lines.next().map(str::trim) != Some(HEADER) {
        return (Vec::new(), 0);
    }
    let mut skipped = 0;
    let mut specs = Vec::new();
    for line in lines.filter(|l| !l.trim().is_empty()) {
        match PipelineSpec::parse(line) {
            Some(s) if !specs.contains(&s) => specs.push(s),
            Some(_) => {}
            None => skipped += 1,
        }
    }
    (specs, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skinned() -> VertexLayout {
        VertexLayout::packed(&[
            (VertexSemantic::Position, VertexFormat::Float3),
            (VertexSemantic::Normal, VertexFormat::Float3),
            (VertexSemantic::TexCoord(0), VertexFormat::Half2),
            (VertexSemantic::BlendWeight, VertexFormat::UNorm4x8),
            (VertexSemantic::BlendIndices, VertexFormat::UInt4x8),
            (VertexSemantic::Color, VertexFormat::Bgra8),
        ])
        .unwrap()
    }

    fn specs() -> Vec<PipelineSpec> {
        vec![
            PipelineSpec::Draw {
                layout: skinned(),
                topology: Topology::TriangleStrip,
                strip_index: Some(IndexFormat::U16),
                blend: PipelineBlend::Translucent,
                double_sided: true,
                depth_bias: false,
            },
            PipelineSpec::Shadow {
                layout: skinned(),
                topology: Topology::TriangleList,
                strip_index: None,
                masked: true,
            },
        ]
    }

    #[test]
    fn every_spec_reads_back_from_its_line() {
        for s in specs() {
            let line = s.to_line();
            assert_eq!(PipelineSpec::parse(&line).as_ref(), Some(&s), "{line}");
        }
        assert_eq!(
            specs()[1].to_line(),
            "shadow triangles - 1 40 pos:f3:0,nrm:f3:12,tc0:h2:24,bw:un4:28,bi:u4:32,col:bgra:36"
        );
    }

    #[test]
    fn a_file_round_trips_sorted_and_once_each() {
        let mut twice = specs();
        twice.extend(specs());
        let text = write_specs(&twice);
        assert!(text.starts_with("m2 pipelines 1\n"));
        assert_eq!(text.lines().count(), 3);
        let (back, skipped) = read_specs(&text);
        assert_eq!(skipped, 0);
        assert_eq!(back.len(), 2);
        assert!(specs().iter().all(|s| back.contains(s)));
        // The same set writes the same file whatever its order.
        let mut reversed = specs();
        reversed.reverse();
        assert_eq!(write_specs(&reversed), text);
    }

    #[test]
    fn bad_lines_and_other_files_are_skipped() {
        let good = specs()[0].to_line();
        let text =
            format!("{HEADER}\n{good}\ndraw sideways - opaque 0 0 12 pos:f3:0\nnonsense\n\n");
        let (back, skipped) = read_specs(&text);
        assert_eq!(back, vec![specs()[0].clone()]);
        assert_eq!(skipped, 2);
        // An element past its stride is no layout.
        assert!(PipelineSpec::parse("shadow triangles - 0 8 pos:f3:0").is_none());
        // Another header: nothing read.
        assert_eq!(
            read_specs(&format!("m2 pipelines 9\n{good}\n")),
            (Vec::new(), 0)
        );
    }
}
