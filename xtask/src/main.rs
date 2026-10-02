use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use m2_hook::{Status, SymbolTable};

fn main() -> anyhow::Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("symbols") => symbols(),
        _ => bail!("usage: cargo xtask symbols"),
    }
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Validates `symbols/*.toml` and prints how many functions are at each status.
fn symbols() -> anyhow::Result<()> {
    let table =
        SymbolTable::load_dir(&repo_root().join("symbols")).context("loading symbol tables")?;
    println!("{} functions", table.len());
    for status in [
        Status::Found,
        Status::Named,
        Status::Translated,
        Status::Verified,
    ] {
        let count = table.functions().filter(|f| f.status == status).count();
        println!("  {status:?}: {count}");
    }
    Ok(())
}
