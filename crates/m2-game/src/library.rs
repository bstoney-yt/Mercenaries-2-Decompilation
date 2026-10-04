//! The game's scripts as modules: which script `import("MrxUtil")` means.
//!
//! Scripts load each other by module name (`import`, `inherit`,
//! `dynamic_import`), but the archives key scripts by a hash of their name
//! that is not solved. Each script runs in its own environment and defines
//! its module's functions there as globals, and other scripts call them as
//! `MrxUtil.GetRandomTableElement`. So a module is the script that defines
//! most of the members other scripts use on that name, among the scripts
//! that don't themselves load it.

use std::collections::{BTreeMap, BTreeSet};

use m2_assets::lua::{Chunk, NameUse};

/// One compiled script and what it defines and loads.
#[derive(Clone, Debug)]
pub struct ScriptInfo {
    /// The asset's name hash.
    pub hash: u32,
    pub chunk: Chunk,
    /// Globals the script assigns: its module's members.
    pub defines: BTreeSet<String>,
    /// Module names it loads, lower case.
    pub loads: BTreeSet<String>,
}

/// Every script, indexed for module lookups.
#[derive(Clone, Debug, Default)]
pub struct Library {
    scripts: Vec<ScriptInfo>,
    /// Lower-case module name to the members scripts use on it.
    used: BTreeMap<String, BTreeSet<String>>,
}

/// Functions that load a module by name.
pub const LOADERS: [&str; 3] = ["import", "inherit", "dynamic_import"];

impl Library {
    pub fn add(&mut self, hash: u32, chunk: Chunk) {
        let mut defines = BTreeSet::new();
        for (u, name) in chunk.main.names(chunk.version) {
            match u {
                NameUse::SetGlobal => {
                    defines.insert(name);
                }
                NameUse::Member => {
                    if let Some((module, member)) = name.split_once('.') {
                        self.used
                            .entry(module.to_ascii_lowercase())
                            .or_default()
                            .insert(member.to_owned());
                    }
                }
                _ => {}
            }
        }
        let loads = chunk
            .main
            .calls()
            .into_iter()
            .filter(|c| LOADERS.contains(&c.callee.as_str()))
            .filter_map(|c| c.first)
            .map(|n| n.to_ascii_lowercase())
            .collect();
        self.scripts.push(ScriptInfo {
            hash,
            chunk,
            defines,
            loads,
        });
    }

    pub fn scripts(&self) -> &[ScriptInfo] {
        &self.scripts
    }

    pub fn by_hash(&self, hash: u32) -> Option<&ScriptInfo> {
        self.scripts.iter().find(|s| s.hash == hash)
    }

    /// The script that is module `name` (any case), with how many of the
    /// members used on it it defines, out of how many. `None` when no
    /// script defines any of them (or nothing is used on it).
    pub fn resolve(&self, name: &str) -> Option<(&ScriptInfo, usize, usize)> {
        let key = name.to_ascii_lowercase();
        let used = self.used.get(&key)?;
        self.scripts
            .iter()
            .filter(|s| !s.loads.contains(&key))
            .map(|s| {
                let hits = used.iter().filter(|m| s.defines.contains(*m)).count();
                (s, hits)
            })
            .filter(|&(_, hits)| hits > 0)
            // Most members used; then the script defining least else.
            .max_by(|(a, ha), (b, hb)| {
                ha.cmp(hb)
                    .then_with(|| b.defines.len().cmp(&a.defines.len()))
                    .then_with(|| b.hash.cmp(&a.hash))
            })
            .map(|(s, hits)| (s, hits, used.len()))
    }

    /// Every module name some script loads, lower case.
    pub fn module_names(&self) -> BTreeSet<&str> {
        self.scripts
            .iter()
            .flat_map(|s| s.loads.iter().map(String::as_str))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// `luac5.1 -s` of our own test modules; sources in `tests/data/`.
    pub(crate) fn library() -> Library {
        let mut lib = Library::default();
        for (hash, bytes) in [
            (1, &include_bytes!("../tests/data/util.luac")[..]),
            (2, include_bytes!("../tests/data/task.luac")),
            (3, include_bytes!("../tests/data/contract.luac")),
            (4, include_bytes!("../tests/data/user.luac")),
        ] {
            lib.add(hash, Chunk::parse(bytes).unwrap());
        }
        lib
    }

    #[test]
    fn modules_are_the_scripts_defining_what_others_use() {
        let lib = library();
        assert_eq!(
            lib.resolve("Util").map(|(s, h, n)| (s.hash, h, n)),
            Some((1, 2, 2))
        );
        // The contract defines Activated too, but it inherits Task.
        assert_eq!(lib.resolve("task").map(|(s, _, _)| s.hash), Some(2));
        assert!(lib.resolve("Nothing").is_none());
        assert_eq!(
            lib.module_names().into_iter().collect::<Vec<_>>(),
            ["task", "util"]
        );
    }
}
