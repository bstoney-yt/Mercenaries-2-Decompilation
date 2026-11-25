//! Render pipelines built ahead, under the loading screen, instead of at
//! the first draw that needs each.
//!
//! The wgpu backend builds a pipeline (and compiles its vertex layout's
//! shader) the first time a draw needs it. Measured on the merge queue's
//! skip-to run (main cbc9b004): the first frame built +7 in 3.3-3.9 s,
//! and +2 took 2239 ms right after "mission: controls on", in play. So:
//!
//! - every pipeline a run builds is written down ([`save`]) in the
//!   settings' folder (`pipelines.txt`, the port's own text form,
//!   [`m2_render::pipeline_spec`]; nothing from the game's files);
//! - the next run reads them ([`start`]) and builds them before its
//!   scene's first draw: offscreen in one pass at the start (no screen to
//!   keep moving), in the window a few each frame within [`WARM_BUDGET`]
//!   ([`step`]) for as long as play hasn't begun (the start and loading
//!   screens cover it) or a loading screen is up ([`screen`]);
//! - where the driver keeps a pipeline cache (Vulkan), its data is saved
//!   beside them (`pipelines-<adapter key>.bin`) and handed back on the
//!   next run, so the warm-up itself compiles less.
//!
//! A first run has no list: there is no built-in set, since the keys are
//! made of the vertex layouts the game's models bring, known only once
//! they load. It builds as before and leaves the list for the next.
//!
//! Provenance: port-side and invented throughout. The 360 ships its
//! shaders precompiled and builds nothing at draw time; this changes when
//! the port's pipelines are built, never what is drawn.
//! `--pipeline-warm-check` ([`verdict`]) is the check: no pipeline built
//! by a draw once the scripts have given her the controls, whatever is on
//! screen, nor once a loading screen has closed with the screen down.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use m2_render::gpu::WgpuBackend;
use m2_render::pipeline_spec::{read_specs, write_specs};
use m2_render::{PipelineSpec, RenderBackend};

/// The list's file name, in the cache folder.
pub const LIST: &str = "pipelines.txt";

/// Time a frame under the loading screen may spend building ahead
/// (invented: the screen's movie keeps moving at a few frames a second
/// with this, where the 3.4 s first frame froze it). At least one
/// pipeline is built each such frame, however long it takes.
pub const WARM_BUDGET: Duration = Duration::from_millis(50);

/// The start of the driver cache's file: a name, the data's length and
/// its FNV-1a hash, both little-endian, so a cut-short or damaged file is
/// never handed to the driver.
const BLOB_MAGIC: &[u8; 4] = b"M2PC";

fn fnv1a64(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The driver cache's file for `data`.
pub fn encode_blob(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 20);
    out.extend_from_slice(BLOB_MAGIC);
    out.extend_from_slice(&(data.len() as u64).to_le_bytes());
    out.extend_from_slice(&fnv1a64(data).to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// The data an [`encode_blob`] file holds, or none when it isn't whole.
pub fn decode_blob(file: &[u8]) -> Option<&[u8]> {
    let rest = file.strip_prefix(BLOB_MAGIC.as_slice())?;
    let (len, rest) = rest.split_at_checked(8)?;
    let (hash, data) = rest.split_at_checked(8)?;
    let len = u64::from_le_bytes(len.try_into().ok()?);
    let hash = u64::from_le_bytes(hash.try_into().ok()?);
    (data.len() as u64 == len && fnv1a64(data) == hash).then_some(data)
}

/// The list to write: those read at the start and those built since,
/// each once (sorted by [`write_specs`]).
pub fn merged(known: &[PipelineSpec], built: &[PipelineSpec]) -> String {
    write_specs(known.iter().chain(built))
}

/// When play has begun, and what draws built before and after: the
/// [`verdict`]'s counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Watch {
    screen_up: bool,
    seen_up: bool,
    /// Play has begun: the loading screen went down after being up, or
    /// the scripts gave her the controls.
    live: bool,
    /// The scripts have given her the controls (once is enough).
    controls_given: bool,
    /// Pipelines draws built in play with the screen down, and over how
    /// many frames.
    pub in_play: u32,
    pub in_play_frames: u32,
    /// Pipelines draws built before play or under the screen.
    pub covered: u32,
}

impl Watch {
    pub fn screen(&mut self, up: bool) {
        if self.seen_up && !up {
            self.live = true;
        }
        self.seen_up |= up;
        self.screen_up = up;
    }

    pub fn controls(&mut self, on: bool) {
        self.live |= on;
        self.controls_given |= on;
    }

    /// A frame's draws built `made` pipelines.
    pub fn drew(&mut self, made: u32) {
        if made == 0 {
            return;
        }
        // Once she has had the controls, any build is in play, even under
        // a later loading screen; before, only with the screen down after
        // one has closed.
        if self.controls_given || (self.live && !self.screen_up) {
            self.in_play += made;
            self.in_play_frames += 1;
        } else {
            self.covered += made;
        }
    }

    pub fn live(&self) -> bool {
        self.live
    }

    /// Whether building ahead is covered now: before play begins (the
    /// start and loading screens are up) or under a loading screen.
    pub fn covered_now(&self) -> bool {
        !self.live || self.screen_up
    }
}

#[derive(Debug, Default)]
struct State {
    dir: Option<PathBuf>,
    started: bool,
    /// The list read at the start (kept in the next list), and those of
    /// it not yet built.
    known: Vec<PipelineSpec>,
    pending: VecDeque<PipelineSpec>,
    /// Built ahead: how many, in what time.
    warmed: u32,
    warm_took: Duration,
    /// What the driver's cache was at the start.
    driver_cache: String,
    watch: Watch,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// Where the list and the driver's cache are kept (the settings' folder,
/// or `--pipeline-cache-dir`).
pub fn set_dir(dir: PathBuf) {
    STATE.with_borrow_mut(|s| s.dir = Some(dir));
}

/// A backend starts: reads the list, and starts the driver's cache from
/// its file where the device keeps one. `at_once` (offscreen, where no
/// screen has to keep moving): builds the whole list now, before the
/// scene's first draw; otherwise [`step`] builds it frame by frame.
pub fn start(backend: &mut WgpuBackend, at_once: bool) {
    STATE.with_borrow_mut(|s| {
        let Some(dir) = s.dir.clone() else { return };
        s.started = true;
        let list = dir.join(LIST);
        match std::fs::read_to_string(&list) {
            Ok(text) => {
                let (specs, skipped) = read_specs(&text);
                eprintln!(
                    "pipelines: {} to build ahead from {} ({skipped} lines skipped)",
                    specs.len(),
                    list.display()
                );
                s.pending = specs.iter().cloned().collect();
                s.known = specs;
            }
            Err(_) => eprintln!(
                "pipelines: no list at {} (a first run: built as drawn)",
                list.display()
            ),
        }
        s.driver_cache = match backend.pipeline_cache_key() {
            None => "none on this device".to_owned(),
            Some(key) => {
                let file = dir.join(format!("pipelines-{key}.bin"));
                let bytes = std::fs::read(&file).ok();
                let data = bytes.as_deref().and_then(decode_blob);
                backend.use_pipeline_cache(data);
                match (&bytes, data) {
                    (None, _) => "started empty".to_owned(),
                    (Some(_), Some(d)) => format!("{} bytes read", d.len()),
                    (Some(_), None) => "file damaged: started empty".to_owned(),
                }
            }
        };
        eprintln!("pipelines: the driver's cache: {}", s.driver_cache);
    });
    if at_once {
        warm(backend, None, "before the first frame");
    }
}

/// Builds pipelines of the list, within `budget` (one at least) or all of
/// them, and logs what it built.
fn warm(backend: &mut WgpuBackend, budget: Option<Duration>, when: &str) {
    let built = STATE.with_borrow_mut(|s| {
        if s.pending.is_empty() {
            return None;
        }
        let started = Instant::now();
        let mut built = 0u32;
        while let Some(spec) = s.pending.pop_front() {
            match backend.prepare_pipeline(&spec) {
                Ok(new) => built += u32::from(new),
                Err(e) => eprintln!("pipelines: could not build {}: {e}", spec.to_line()),
            }
            if budget.is_some_and(|b| started.elapsed() >= b) {
                break;
            }
        }
        Some((built, s.pending.len()))
    });
    let Some((built, left)) = built else { return };
    // Taken here, so the draws' own count (`note_pipelines`) has only
    // what they built.
    let work = backend.take_pipeline_work();
    crate::spans::add("warm pipelines", work.took);
    STATE.with_borrow_mut(|s| {
        s.warmed += built;
        s.warm_took += work.took;
    });
    eprintln!(
        "pipelines: warmed +{built} (left {left}) in {:.0} ms {when}",
        work.took.as_secs_f32() * 1000.0
    );
    if left == 0 {
        // All built: the driver's cache now holds them too.
        save(backend);
    }
}

/// The loading screen is up (or not) this frame.
pub fn screen(up: bool) {
    STATE.with_borrow_mut(|s| s.watch.screen(up));
}

/// The scripts gave her the controls (or took them).
pub fn controls(on: bool) {
    STATE.with_borrow_mut(|s| s.watch.controls(on));
}

/// Before a frame's draws: while building ahead is covered (play not
/// begun, or a loading screen up), builds pipelines of the list within
/// [`WARM_BUDGET`] (one at least).
pub fn step(backend: &mut WgpuBackend) {
    if STATE.with_borrow(|s| s.watch.covered_now()) {
        warm(
            backend,
            Some(WARM_BUDGET),
            "under the start or loading screen",
        );
    }
}

/// A frame's draws built `made` pipelines: counted for the check, and the
/// list (and the driver's cache) saved when there were any.
pub fn drew(backend: &WgpuBackend, made: u32) {
    let save_now = STATE.with_borrow_mut(|s| {
        s.watch.drew(made);
        s.started && made > 0
    });
    if save_now {
        save(backend);
    }
}

/// Writes the list (those read and those built) and the driver's cache.
pub fn save(backend: &WgpuBackend) {
    STATE.with_borrow(|s| {
        let Some(dir) = s.dir.as_ref().filter(|_| s.started) else {
            return;
        };
        let text = merged(&s.known, &backend.used_pipelines());
        if let Err(e) = write_whole(&dir.join(LIST), text.as_bytes()) {
            eprintln!("pipelines: could not save the list: {e}");
        }
        if let (Some(key), Some(data)) =
            (backend.pipeline_cache_key(), backend.pipeline_cache_data())
        {
            let file = dir.join(format!("pipelines-{key}.bin"));
            if let Err(e) = write_whole(&file, &encode_blob(&data)) {
                eprintln!("pipelines: could not save the driver's cache: {e}");
            }
        }
    });
}

/// Writes `path` whole or not at all: a temporary file, then renamed.
fn write_whole(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `--pipeline-warm-check`'s verdict on the run so far.
pub fn verdict() -> (bool, String) {
    STATE.with_borrow(|s| {
        verdict_of(
            &s.watch,
            s.known.len(),
            s.warmed,
            s.warm_took,
            &s.driver_cache,
        )
    })
}

/// The verdict: play must have begun, and no draw may have built a
/// pipeline once she has had the controls, nor with the screen down once
/// a loading screen has closed.
pub fn verdict_of(
    w: &Watch,
    listed: usize,
    warmed: u32,
    took: Duration,
    driver_cache: &str,
) -> (bool, String) {
    let cache = if listed == 0 {
        "cold (no list)".to_owned()
    } else {
        format!("warm ({listed} listed)")
    };
    let summary = format!(
        "cache {cache}; warmed +{warmed} in {:.0} ms ahead; \
         draws built +{} before play or under the screen; driver cache {driver_cache}",
        took.as_secs_f32() * 1000.0,
        w.covered
    );
    if !w.live() {
        return (
            false,
            format!("FAIL play never began (no screen closed, no controls on); {summary}"),
        );
    }
    if w.in_play > 0 {
        return (
            false,
            format!(
                "FAIL draws built +{} in play over {} frames; {summary}",
                w.in_play, w.in_play_frames
            ),
        );
    }
    (true, format!("PASS no pipeline built in play; {summary}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_render::buffer::{VertexFormat, VertexLayout, VertexSemantic};
    use m2_render::{PipelineBlend, Topology};

    fn spec(blend: PipelineBlend) -> PipelineSpec {
        PipelineSpec::Draw {
            layout: VertexLayout::packed(&[(VertexSemantic::Position, VertexFormat::Float3)])
                .unwrap(),
            topology: Topology::TriangleList,
            strip_index: None,
            blend,
            double_sided: false,
            depth_bias: false,
        }
    }

    #[test]
    fn the_driver_cache_file_round_trips_and_refuses_damage() {
        let data = b"driver bytes".to_vec();
        let file = encode_blob(&data);
        assert_eq!(decode_blob(&file), Some(data.as_slice()));
        // Cut short, a byte changed, another file: none.
        assert_eq!(decode_blob(&file[..file.len() - 1]), None);
        let mut changed = file.clone();
        *changed.last_mut().unwrap() ^= 1;
        assert_eq!(decode_blob(&changed), None);
        assert_eq!(decode_blob(b"PNG..."), None);
        assert_eq!(decode_blob(&encode_blob(&[])), Some(&[][..]));
    }

    #[test]
    fn the_list_keeps_what_was_read_and_adds_what_was_built() {
        let read = vec![spec(PipelineBlend::Opaque)];
        let built = vec![spec(PipelineBlend::Opaque), spec(PipelineBlend::Additive)];
        let text = merged(&read, &built);
        let (back, skipped) = read_specs(&text);
        assert_eq!(skipped, 0);
        assert_eq!(back.len(), 2);
        assert!(back.contains(&spec(PipelineBlend::Additive)));
    }

    #[test]
    fn a_list_saved_to_disk_reads_back() {
        let dir = std::env::temp_dir().join(format!("m2-warmup-test-{}", std::process::id()));
        let file = dir.join(LIST);
        let text = merged(
            &[spec(PipelineBlend::Masked)],
            &[spec(PipelineBlend::Translucent)],
        );
        write_whole(&file, text.as_bytes()).unwrap();
        let (back, _) = read_specs(&std::fs::read_to_string(&file).unwrap());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(back.len(), 2);
        assert!(back.contains(&spec(PipelineBlend::Masked)));
        assert!(back.contains(&spec(PipelineBlend::Translucent)));
    }

    #[test]
    fn only_builds_in_play_with_the_screen_down_fail_the_check() {
        let mut w = Watch::default();
        // Before any screen: covered, play not begun.
        w.drew(7);
        w.screen(true);
        w.drew(4);
        assert_eq!((w.covered, w.in_play), (11, 0));
        assert!(!verdict_of(&w, 0, 0, Duration::ZERO, "none").0);
        // The screen goes: play has begun; nothing built since: PASS.
        w.screen(false);
        w.drew(0);
        let (ok, line) = verdict_of(&w, 56, 49, Duration::from_millis(3300), "none");
        assert!(ok, "{line}");
        assert!(line.starts_with("PASS no pipeline built in play; cache warm (56 listed)"));
        // Up again over a later load: still covered.
        w.screen(true);
        w.drew(3);
        assert_eq!(w.in_play, 0);
        // Down, and a draw builds two: FAIL.
        w.screen(false);
        w.drew(2);
        let (ok, line) = verdict_of(&w, 56, 49, Duration::ZERO, "none");
        assert!(!ok);
        assert!(
            line.starts_with("FAIL draws built +2 in play over 1 frames"),
            "{line}"
        );
    }

    #[test]
    fn after_the_controls_any_build_fails_even_under_a_screen() {
        let mut w = Watch::default();
        w.screen(true);
        w.drew(7);
        w.controls(true);
        // Up again over a later load, with the controls given: in play.
        w.screen(true);
        w.drew(2);
        assert_eq!((w.covered, w.in_play), (7, 2));
        assert!(!verdict_of(&w, 50, 50, Duration::ZERO, "none").0);
    }

    #[test]
    fn building_ahead_is_covered_until_play_begins_or_under_a_screen() {
        let mut w = Watch::default();
        // At the start, before any screen (offscreen skip-to): covered.
        assert!(w.covered_now());
        w.controls(true);
        assert!(!w.covered_now());
        w.screen(true);
        assert!(w.covered_now());
        w.screen(false);
        assert!(!w.covered_now());
    }

    #[test]
    fn controls_on_begin_play_without_a_screen() {
        let mut w = Watch::default();
        w.controls(true);
        w.drew(2);
        assert_eq!(w.in_play, 2);
        assert!(!verdict_of(&w, 0, 0, Duration::ZERO, "none").0);
    }
}
