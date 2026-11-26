//! One GitHub issue per distinct break, deduplicated by contract and
//! failure kind, and one index comment on the pinned known-bugs issue.
//!
//! Each issue's body carries hidden markers: its key
//! (`<!-- nightly-key: PirCon001/stuck -->`), when it was first and last
//! seen and how often. A break seen again edits that body (silent: no
//! notification); a closed issue whose break comes back is reopened with
//! one comment. The index is one comment on the known-bugs issue, marked
//! `<!-- nightly-autoplay-index -->`, edited only when its list changes.
//!
//! Text only: contract ids, failure kinds, numbers and our own source
//! locations. No log lines, game text, footage or paths on the VM.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use serde::Deserialize;

use crate::detect::Kind;
use crate::night::{Night, Outcome, short};

pub const LABEL: &str = "nightly-autoplay";
pub const INDEX_MARKER: &str = "<!-- nightly-autoplay-index -->";

/// The dedupe key of a break.
pub fn key(contract: &str, kind: Kind) -> String {
    format!("{contract}/{}", kind.name())
}

/// The value of `<!-- name: value -->` in `body`.
pub fn marker<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<!-- {name}: ");
    let i = body.find(&open)? + open.len();
    let rest = &body[i..];
    Some(rest[..rest.find(" -->")?].trim())
}

/// An issue already on GitHub with the label.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Existing {
    pub number: u64,
    /// `OPEN` or `CLOSED`.
    pub state: String,
    pub title: String,
    pub body: String,
}

impl Existing {
    pub fn key(&self) -> Option<&str> {
        marker(&self.body, "nightly-key")
    }
    pub fn open(&self) -> bool {
        self.state.eq_ignore_ascii_case("open")
    }
}

/// What to do on GitHub.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Create {
        key: String,
        title: String,
        body: String,
    },
    /// Rewrite a body (the break seen again).
    Edit {
        number: u64,
        key: String,
        body: String,
    },
    /// A closed issue's break came back.
    Reopen {
        number: u64,
        key: String,
        body: String,
        comment: String,
    },
}

/// One break this night: contract, kind, the detail line.
#[derive(Debug, Clone, PartialEq)]
pub struct Break {
    pub contract: String,
    pub kind: Kind,
    pub detail: String,
}

/// The night's distinct breaks, one per contract and issue kind.
pub fn breaks(n: &Night) -> Vec<Break> {
    let mut out: Vec<Break> = Vec::new();
    for r in n.results.iter().filter(|r| r.outcome == Outcome::Fail) {
        for &kind in &r.issue_kinds {
            if out
                .iter()
                .any(|b| b.contract == r.contract && b.kind == kind)
            {
                continue;
            }
            let detail = r
                .failures
                .iter()
                .find(|f| f.kind == kind)
                .map(|f| f.detail.clone())
                .unwrap_or_default();
            out.push(Break {
                contract: r.contract.clone(),
                kind,
                detail,
            });
        }
    }
    out
}

pub fn title(b: &Break) -> String {
    format!("Nightly autoplay: {} {}", b.contract, b.kind.name())
}

/// What each kind means, for the issue body.
fn meaning(kind: Kind) -> &'static str {
    match kind {
        Kind::Panic => "m2-alpha panicked during the autoplay run.",
        Kind::Crash => "m2-alpha exited abnormally (crash exit code).",
        Kind::Error => "m2-alpha stopped on an error of its own.",
        Kind::Hang => "m2-alpha stopped writing its log: the process hung and was stopped.",
        Kind::FellThrough => "The hero fell through the world (or her position was lost).",
        Kind::Stuck => {
            "The contract's objectives and mission state stopped changing: autoplay is stuck."
        }
        Kind::FpsCliff => "The frame time stayed over its ceiling for a sustained stretch.",
        Kind::Timeout => "The run went past its wall-clock limit.",
        Kind::NotCompleted => "The run ended without the contract completed.",
    }
}

/// An issue body: what broke, where it was seen, how to reproduce it.
pub fn body(
    b: &Break,
    first: (&str, &str),
    last: (&str, &str),
    seen: u64,
    known_bugs: u64,
) -> String {
    let skip = if b.contract == crate::contracts::FIRST {
        String::new()
    } else {
        format!(" --skip-to {}", b.contract)
    };
    format!(
        "<!-- nightly-key: {key} -->\n\
**Contract:** {c}\n**Failure:** `{kind}`: {meaning}\n\n\
**Latest:** {detail}\n\n\
| | night | head |\n|---|---|---|\n| first seen | {f0} | {f1} |\n| last seen | {l0} | {l1} |\n\n\
Seen in {seen} nightly run(s).\n\n\
Reproduce (the nightly's own command):\n\n\
```\nm2-alpha --install <install dir> --mission --autoplay --contracts 1 --mission-seconds 1500 --size 640x360 --screenshot <out>.ppm{skip}\n```\n\n\
The run's log stays on the dev VM, in the nightly folder `{l0}/{c}`. \
Filed by the nightly autoplay runner (`tools/nightly-autoplay`, `docs/nightly-autoplay.md`); listed in #{known_bugs}.\n\n\
<!-- nightly-first: {f0} {f1} -->\n<!-- nightly-last: {l0} {l1} -->\n<!-- nightly-seen: {seen} -->\n",
        key = key(&b.contract, b.kind),
        c = b.contract,
        kind = b.kind.name(),
        meaning = meaning(b.kind),
        detail = b.detail,
        f0 = first.0,
        f1 = short(first.1),
        l0 = last.0,
        l1 = short(last.1),
    )
}

/// Plans the night's issue changes against what is on GitHub. A break
/// already marked as seen in this night (the runner run twice) is left
/// alone.
pub fn plan(n: &Night, existing: &[Existing], known_bugs: u64) -> Vec<Action> {
    let mut actions = Vec::new();
    let this = (n.run_id.as_str(), n.head.as_str());
    for b in breaks(n) {
        let k = key(&b.contract, b.kind);
        // The open one if any, else the newest closed one.
        let found = existing
            .iter()
            .filter(|e| e.key() == Some(k.as_str()))
            .max_by_key(|e| (e.open(), e.number));
        match found {
            None => actions.push(Action::Create {
                title: title(&b),
                body: body(&b, this, this, 1, known_bugs),
                key: k,
            }),
            Some(e) => {
                let last = marker(&e.body, "nightly-last").unwrap_or("");
                if last.split(' ').next() == Some(n.run_id.as_str()) {
                    continue;
                }
                let first = marker(&e.body, "nightly-first").unwrap_or("? ?");
                let (f0, f1) = first.split_once(' ').unwrap_or((first, "?"));
                let seen = marker(&e.body, "nightly-seen")
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(1)
                    + 1;
                let new_body = body(&b, (f0, f1), this, seen, known_bugs);
                if e.open() {
                    actions.push(Action::Edit {
                        number: e.number,
                        key: k,
                        body: new_body,
                    });
                } else {
                    actions.push(Action::Reopen {
                        number: e.number,
                        key: k,
                        body: new_body,
                        comment: format!(
                            "Seen again in the nightly autoplay run {} (head {}): {}",
                            n.run_id,
                            short(&n.head),
                            b.detail
                        ),
                    });
                }
            }
        }
    }
    actions
}

/// The index comment: one line per open nightly issue, sorted by
/// contract. `issues` are (number, key, first night).
pub fn index_body(issues: &[(u64, String, String)]) -> String {
    let mut rows: Vec<&(u64, String, String)> = issues.iter().collect();
    rows.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    let mut s = format!(
        "{INDEX_MARKER}\n**Nightly autoplay breaks** (one issue per contract and failure kind, \
kept up to date by the nightly runner; this comment is edited only when the list changes):\n\n"
    );
    if rows.is_empty() {
        s.push_str("None open.\n");
    }
    for (n, k, first) in rows {
        let (c, kind) = k.split_once('/').unwrap_or((k, ""));
        let n = if *n == 0 {
            "(new)".to_string()
        } else {
            n.to_string()
        };
        s.push_str(&format!("- #{n} {c} `{kind}` (since {first})\n"));
    }
    s
}

/// The open nightly issues after the plan ran: existing open ones, the
/// reopened ones, and the new ones by their numbers (`created`: key to
/// number; in a dry run, 0).
pub fn open_after(
    existing: &[Existing],
    actions: &[Action],
    created: &[(String, u64)],
    run_id: &str,
) -> Vec<(u64, String, String)> {
    let mut out: Vec<(u64, String, String)> = Vec::new();
    let first = |b: &str| {
        marker(b, "nightly-first")
            .and_then(|f| f.split(' ').next())
            .unwrap_or("?")
            .to_string()
    };
    for e in existing {
        let reopened = actions
            .iter()
            .any(|a| matches!(a, Action::Reopen { number, .. } if *number == e.number));
        if (e.open() || reopened)
            && let Some(k) = e.key()
        {
            out.push((e.number, k.to_string(), first(&e.body)));
        }
    }
    for (k, n) in created {
        out.push((*n, k.clone(), run_id.to_string()));
    }
    out
}

// ---- GitHub, through the gh CLI -------------------------------------------

fn gh(args: &[&str]) -> anyhow::Result<String> {
    let o = Command::new("gh")
        .args(args)
        .output()
        .context("running gh (is the GitHub CLI installed and signed in?)")?;
    if !o.status.success() {
        bail!(
            "gh {}: {}",
            args.first().copied().unwrap_or(""),
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&o.stdout).into_owned())
}

/// The labelled issues on `repo`, open and closed.
pub fn fetch_existing(repo: &str) -> anyhow::Result<Vec<Existing>> {
    let out = gh(&[
        "issue",
        "list",
        "--repo",
        repo,
        "--label",
        LABEL,
        "--state",
        "all",
        "--limit",
        "1000",
        "--json",
        "number,state,title,body",
    ])?;
    Ok(serde_json::from_str(&out)?)
}

#[derive(Debug, Deserialize)]
struct Comment {
    id: u64,
    body: String,
}

/// The index comment on the known-bugs issue: (id, body).
pub fn fetch_index(repo: &str, issue: u64) -> anyhow::Result<Option<(u64, String)>> {
    let out = gh(&[
        "api",
        "--paginate",
        "--slurp",
        &format!("repos/{repo}/issues/{issue}/comments?per_page=100"),
    ])?;
    let pages: Vec<Vec<Comment>> = serde_json::from_str(&out)?;
    Ok(pages
        .into_iter()
        .flatten()
        .find(|c| c.body.contains(INDEX_MARKER))
        .map(|c| (c.id, c.body)))
}

/// Applies the plan and the index; with `dry_run`, prints instead. The
/// lines printed say what was (or would be) done.
pub fn apply(
    repo: &str,
    known_bugs: u64,
    n: &Night,
    existing: &[Existing],
    index: Option<(u64, String)>,
    dry_run: bool,
    tmp: &Path,
) -> anyhow::Result<Vec<String>> {
    let actions = plan(n, existing, known_bugs);
    let mut log = Vec::new();
    let mut created: Vec<(String, u64)> = Vec::new();
    let write = |name: &str, text: &str| -> anyhow::Result<String> {
        let p = tmp.join(name);
        std::fs::write(&p, text)?;
        Ok(p.display().to_string())
    };
    if !dry_run && actions.iter().any(|a| matches!(a, Action::Create { .. })) {
        gh(&[
            "label",
            "create",
            LABEL,
            "--repo",
            repo,
            "--color",
            "b60205",
            "--description",
            "Broken in the nightly autoplay run",
            "--force",
        ])?;
    }
    for a in &actions {
        match a {
            Action::Create { key, title, body } => {
                log.push(format!("create \"{title}\" ({key})"));
                if dry_run {
                    log.push(indent(body));
                    created.push((key.clone(), 0));
                    continue;
                }
                let f = write("issue.md", body)?;
                let url = gh(&[
                    "issue",
                    "create",
                    "--repo",
                    repo,
                    "--title",
                    title,
                    "--body-file",
                    &f,
                    "--label",
                    LABEL,
                    "--label",
                    "bug",
                ])?;
                let number = url
                    .trim()
                    .rsplit('/')
                    .next()
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(0);
                log.push(format!("  -> #{number}"));
                created.push((key.clone(), number));
            }
            Action::Edit { number, key, body } => {
                log.push(format!("edit #{number} ({key}): seen again"));
                if !dry_run {
                    let f = write("issue.md", body)?;
                    gh(&[
                        "issue",
                        "edit",
                        &number.to_string(),
                        "--repo",
                        repo,
                        "--body-file",
                        &f,
                    ])?;
                }
            }
            Action::Reopen {
                number,
                key,
                body,
                comment,
            } => {
                log.push(format!("reopen #{number} ({key}): {comment}"));
                if !dry_run {
                    let f = write("issue.md", body)?;
                    gh(&[
                        "issue",
                        "edit",
                        &number.to_string(),
                        "--repo",
                        repo,
                        "--body-file",
                        &f,
                    ])?;
                    gh(&[
                        "issue",
                        "reopen",
                        &number.to_string(),
                        "--repo",
                        repo,
                        "--comment",
                        comment,
                    ])?;
                }
            }
        }
    }
    let list = index_body(&open_after(existing, &actions, &created, &n.run_id));
    match index {
        Some((_, ref old)) if old.trim() == list.trim() => {
            log.push(format!("index on #{known_bugs}: unchanged"));
        }
        Some((id, _)) => {
            log.push(format!("edit the index comment on #{known_bugs}"));
            if dry_run {
                log.push(indent(&list));
            } else {
                let f = write("index.md", &list)?;
                gh(&[
                    "api",
                    "-X",
                    "PATCH",
                    &format!("repos/{repo}/issues/comments/{id}"),
                    "-F",
                    &format!("body=@{f}"),
                ])?;
            }
        }
        None if actions.is_empty() => {
            log.push(format!("index on #{known_bugs}: nothing to list yet"));
        }
        None => {
            log.push(format!("add the index comment on #{known_bugs}"));
            if dry_run {
                log.push(indent(&list));
            } else {
                let f = write("index.md", &list)?;
                gh(&[
                    "issue",
                    "comment",
                    &known_bugs.to_string(),
                    "--repo",
                    repo,
                    "--body-file",
                    &f,
                ])?;
            }
        }
    }
    Ok(log)
}

fn indent(s: &str) -> String {
    s.lines()
        .map(|l| format!("    | {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}
