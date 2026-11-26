//! `m2-nightly`: the nightly autoplay runner (see the library docs and
//! `docs/nightly-autoplay.md`).

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::{Context, bail};

use m2_nightly::contracts;
use m2_nightly::detect::{Exit, Killed, Thresholds, analyse, issue_kinds};
use m2_nightly::gpu::{BUSY_PREFIXES, now_unix};
use m2_nightly::issues;
use m2_nightly::night::{Night, report, utc_stamp};
use m2_nightly::run::{NightConfig, read_state, run_night};

const USAGE: &str = "\
m2-nightly contracts --install <dir> [--jobs]
    list the contracts (and jobs) in the install's data\\vz.wad, story order
m2-nightly run --exe <m2-alpha.exe> --install <dir> --out <root> [options]
    play each contract by autoplay, one at a time; writes <root>/<run id>/
    --only A,B,..          these contracts, in this order (else every one,
                           least recently played first, from <root>/last-played.json)
    --jobs                 the jobs too
    --deadline-unix <t>    the window's end: the run then playing is stopped
    --max-minutes <m>      ...or this long from now
    --run-id <name>        the night's folder (default: UTC YYYYMMDD-HHMMZ)
    --head <sha>           the commit the exe was built from
    --mission-seconds <s>  autoplay's play limit (default 1500)
    --wall-seconds <s>     each run's wall limit (default 2700)
    --stuck-seconds <s>    no objective change this long is stuck (default 600)
    --hang-seconds <s>     no log output this long is a hang (default 300)
    --floor-y <m>          heights under this are under the world (default -150)
    --tick-ms <ms>         pace ms a tick over this is slow (default 50)
    --fps-floor <fps>      pace fps under this is slow (default 15)
    --slow-lines <n>       slow pace lines in a row for a cliff (default 3)
    --lock <path>          GPU lock file (default C:\\Claude\\scratch\\gpu.lock)
    --busy <a,b|none>      process-name prefixes holding the GPU (default m2-alpha,xenia)
    --free-checks <n>      free GPU checks in a row before starting (default 3)
    --gpu-poll <s>         seconds between GPU checks (default 60)
    --keep-pictures        keep the pictures of passing runs too
    --exe-prefix <arg>     an argument before m2-alpha's own (repeatable; tests)
m2-nightly analyse <run.err> [--exit-code <n>] [--killed wall|hang|stuck|deadline] [--json]
    run the detectors over a log
m2-nightly report <night.json>
    print the night's short report
m2-nightly issues --night <night.json> --repo <owner/name> [--known-bugs 698] [--dry-run] [--offline]
    open or update one issue per break, and the index comment on the known-bugs issue";

fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(e) => {
            eprintln!("m2-nightly: {e:#}");
            ExitCode::from(2)
        }
    }
}

struct Args(std::vec::IntoIter<String>);

impl Args {
    fn value(&mut self, name: &str) -> anyhow::Result<String> {
        self.0
            .next()
            .with_context(|| format!("{name} expects a value"))
    }
    fn num<T: std::str::FromStr>(&mut self, name: &str) -> anyhow::Result<T> {
        let v = self.value(name)?;
        v.parse()
            .ok()
            .with_context(|| format!("{name}: not a number: {v}"))
    }
}

fn list(v: &str) -> Vec<String> {
    v.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

fn run() -> anyhow::Result<ExitCode> {
    let mut all: Vec<String> = std::env::args().skip(1).collect();
    if all.is_empty() || all[0] == "-h" || all[0] == "--help" {
        println!("{USAGE}");
        return Ok(ExitCode::SUCCESS);
    }
    let cmd = all.remove(0);
    let mut a = Args(all.into_iter());
    match cmd.as_str() {
        "contracts" => {
            let (mut install, mut jobs) = (None, false);
            while let Some(x) = a.0.next() {
                match x.as_str() {
                    "--install" => install = Some(PathBuf::from(a.value(&x)?)),
                    "--jobs" => jobs = true,
                    _ => bail!("unknown option {x}\n{USAGE}"),
                }
            }
            let install = install.context("--install is needed")?;
            let ids = contracts::from_install(&install, jobs)?;
            for id in &ids {
                println!("{id}");
            }
            eprintln!("{} found", ids.len());
            Ok(ExitCode::SUCCESS)
        }
        "run" => run_cmd(a),
        "analyse" | "analyze" => {
            let mut path = None;
            let mut exit = Exit::default();
            let mut json = false;
            let mut th = Thresholds::default();
            while let Some(x) = a.0.next() {
                match x.as_str() {
                    "--exit-code" => exit.code = Some(a.num(&x)?),
                    "--killed" => {
                        exit.killed = Some(match a.value(&x)?.as_str() {
                            "wall" => Killed::Wall,
                            "hang" => Killed::Hang,
                            "stuck" => Killed::Stuck,
                            "deadline" => Killed::Deadline,
                            v => bail!("--killed: unknown {v}"),
                        })
                    }
                    "--json" => json = true,
                    "--mission-seconds" => th.mission_seconds = a.num(&x)?,
                    "--stuck-seconds" => th.stuck_seconds = a.num(&x)?,
                    s if s.starts_with("--") => bail!("unknown option {s}\n{USAGE}"),
                    _ => path = Some(x),
                }
            }
            let path = path.context("no log given")?;
            let bytes = std::fs::read(&path).with_context(|| path.clone())?;
            let text = String::from_utf8_lossy(&bytes);
            let r = analyse(text.trim_start_matches('\u{feff}'), &exit, &th);
            if json {
                println!("{}", serde_json::to_string_pretty(&r.failures)?);
            } else {
                println!(
                    "completed {} | longest gap {:.0} s from {:.0} s | lowest y {:?} | worst tick {:?} ms",
                    r.completed, r.longest_gap.0, r.longest_gap.1, r.lowest_y, r.worst_tick_ms
                );
                for f in &r.failures {
                    println!("  {}: {}", f.kind.name(), f.detail);
                }
                let kinds: Vec<&str> = issue_kinds(&r.failures).iter().map(|k| k.name()).collect();
                println!(
                    "issues: {}",
                    if kinds.is_empty() {
                        "none".into()
                    } else {
                        kinds.join(", ")
                    }
                );
            }
            Ok(if r.failures.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            })
        }
        "report" => {
            let p = a.value("report")?;
            let n: Night =
                serde_json::from_str(&std::fs::read_to_string(&p).with_context(|| p.clone())?)?;
            print!("{}", report(&n, 3000));
            Ok(ExitCode::SUCCESS)
        }
        "issues" => {
            let (mut night, mut repo, mut known, mut dry, mut offline) =
                (None, None, 698u64, false, false);
            while let Some(x) = a.0.next() {
                match x.as_str() {
                    "--night" => night = Some(a.value(&x)?),
                    "--repo" => repo = Some(a.value(&x)?),
                    "--known-bugs" => known = a.num(&x)?,
                    "--dry-run" => dry = true,
                    "--offline" => {
                        offline = true;
                        dry = true;
                    }
                    _ => bail!("unknown option {x}\n{USAGE}"),
                }
            }
            let night = night.context("--night is needed")?;
            let repo = repo.context("--repo is needed")?;
            let n: Night = serde_json::from_str(
                &std::fs::read_to_string(&night).with_context(|| night.clone())?,
            )?;
            let (existing, index) = if offline {
                (Vec::new(), None)
            } else {
                (
                    issues::fetch_existing(&repo)?,
                    issues::fetch_index(&repo, known)?,
                )
            };
            let tmp = std::env::temp_dir().join(format!("m2-nightly-{}", std::process::id()));
            std::fs::create_dir_all(&tmp)?;
            let log = issues::apply(&repo, known, &n, &existing, index, dry, &tmp);
            let _ = std::fs::remove_dir_all(&tmp);
            if dry {
                println!("dry run: nothing posted");
            }
            for l in log? {
                println!("{l}");
            }
            Ok(ExitCode::SUCCESS)
        }
        _ => bail!("unknown command {cmd}\n{USAGE}"),
    }
}

fn run_cmd(mut a: Args) -> anyhow::Result<ExitCode> {
    let mut exe = None;
    let mut install = None;
    let mut root = None;
    let mut only: Vec<String> = Vec::new();
    let mut jobs = false;
    let mut deadline = None;
    let mut run_id = None;
    let mut head = "unknown".to_string();
    let mut th = Thresholds::default();
    let mut wall = 2700u64;
    let mut lock = PathBuf::from(r"C:\Claude\scratch\gpu.lock");
    let mut busy: Vec<String> = BUSY_PREFIXES.iter().map(|s| s.to_string()).collect();
    let mut free_checks = 3u32;
    let mut gpu_poll = 60u64;
    let mut keep = false;
    let mut prefix = Vec::new();
    while let Some(x) = a.0.next() {
        match x.as_str() {
            "--exe" => exe = Some(PathBuf::from(a.value(&x)?)),
            "--install" => install = Some(PathBuf::from(a.value(&x)?)),
            "--out" => root = Some(PathBuf::from(a.value(&x)?)),
            "--only" => only = list(&a.value(&x)?),
            "--jobs" => jobs = true,
            "--deadline-unix" => deadline = Some(a.num::<u64>(&x)?),
            "--max-minutes" => {
                let m: u64 = a.num(&x)?;
                let d = now_unix() + m * 60;
                deadline = Some(deadline.map_or(d, |o: u64| o.min(d)));
            }
            "--run-id" => run_id = Some(a.value(&x)?),
            "--head" => head = a.value(&x)?,
            "--mission-seconds" => th.mission_seconds = a.num(&x)?,
            "--wall-seconds" => wall = a.num(&x)?,
            "--stuck-seconds" => th.stuck_seconds = a.num(&x)?,
            "--hang-seconds" => th.hang_seconds = a.num(&x)?,
            "--floor-y" => th.floor_y = a.num(&x)?,
            "--tick-ms" => th.tick_ms_ceiling = a.num(&x)?,
            "--fps-floor" => th.fps_floor = a.num(&x)?,
            "--slow-lines" => th.slow_lines = a.num(&x)?,
            "--lock" => lock = PathBuf::from(a.value(&x)?),
            "--busy" => {
                let v = a.value(&x)?;
                busy = if v == "none" { Vec::new() } else { list(&v) };
            }
            "--free-checks" => free_checks = a.num(&x)?,
            "--gpu-poll" => gpu_poll = a.num(&x)?,
            "--keep-pictures" => keep = true,
            "--exe-prefix" => prefix.push(a.value(&x)?),
            _ => bail!("unknown option {x}\n{USAGE}"),
        }
    }
    let exe = exe.context("--exe is needed")?;
    let install = install.context("--install is needed")?;
    let root = root.context("--out is needed")?;
    std::fs::create_dir_all(&root)?;
    let state = root.join("last-played.json");
    let listed;
    let order = if only.is_empty() {
        let ids = contracts::from_install(&install, jobs)?;
        listed = ids.len();
        contracts::schedule(&ids, &read_state(&state))
    } else {
        listed = only.len();
        only
    };
    let run_id = run_id.unwrap_or_else(|| utc_stamp(now_unix()));
    let cfg = NightConfig {
        exe,
        exe_prefix_args: prefix,
        install,
        out: root.join(&run_id),
        run_id,
        head,
        contracts: order,
        contracts_listed: listed,
        thresholds: th,
        wall_seconds: wall,
        deadline_unix: deadline,
        min_start_seconds: 300,
        lock,
        busy,
        free_checks,
        gpu_poll_seconds: gpu_poll,
        stale_seconds: 8 * 3600,
        poll_seconds: 5,
        stuck_check_seconds: 60,
        keep_pictures: keep,
        state: Some(state),
    };
    let night = run_night(&cfg)?;
    print!("{}", report(&night, 3000));
    println!("{}", cfg.out.join("night.json").display());
    Ok(ExitCode::SUCCESS)
}
