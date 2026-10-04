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
    /// Its string constants, lower case.
    pub strings: Vec<String>,
}

/// Every script, indexed for module lookups.
#[derive(Clone, Debug, Default)]
pub struct Library {
    scripts: Vec<ScriptInfo>,
    /// Lower-case module name to the members scripts use on it.
    used: BTreeMap<String, BTreeSet<String>>,
    /// Lower-case module name to (script index, members it defines),
    /// worked out on first use.
    assigned: std::cell::OnceCell<BTreeMap<String, (usize, usize)>>,
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
        let strings = chunk
            .main
            .strings()
            .into_iter()
            .map(str::to_ascii_lowercase)
            .collect();
        self.scripts.push(ScriptInfo {
            hash,
            chunk,
            defines,
            loads,
            strings,
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
    ///
    /// Every module is scored against every script that doesn't load it:
    /// one point per member used on the module that the script defines,
    /// and two more when the script's strings name the module (scripts
    /// often name themselves in their messages, which settles classes
    /// whose few used members, such as `Activated`, many scripts define).
    /// The best pairs are taken first and a script is one module only.
    pub fn resolve(&self, name: &str) -> Option<(&ScriptInfo, usize, usize)> {
        let key = name.to_ascii_lowercase();
        let &(index, hits) = self.assigned.get_or_init(|| self.assign()).get(&key)?;
        Some((&self.scripts[index], hits, self.used.get(&key)?.len()))
    }

    fn assign(&self) -> BTreeMap<String, (usize, usize)> {
        // (score, hits, module, script index)
        let mut pairs = Vec::new();
        for (module, used) in &self.used {
            for (i, s) in self.scripts.iter().enumerate() {
                if s.loads.contains(module) {
                    continue;
                }
                let hits = used.iter().filter(|m| s.defines.contains(*m)).count();
                if hits == 0 {
                    continue;
                }
                let named = s.strings.iter().any(|t| mentions(t, module));
                pairs.push((hits + if named { 2 } else { 0 }, hits, module, i));
            }
        }
        // Best score; then the script defining least else; then stable.
        pairs.sort_by(|a, b| {
            b.0.cmp(&a.0)
                .then_with(|| {
                    self.scripts[a.3]
                        .defines
                        .len()
                        .cmp(&self.scripts[b.3].defines.len())
                })
                .then_with(|| a.2.cmp(b.2))
                .then_with(|| self.scripts[a.3].hash.cmp(&self.scripts[b.3].hash))
        });
        let mut out = BTreeMap::new();
        let mut taken = BTreeSet::new();
        for (_, hits, module, i) in pairs {
            if out.contains_key(module) || taken.contains(&i) {
                continue;
            }
            taken.insert(i);
            out.insert(module.clone(), (i, hits));
        }
        out
    }

    /// Every module name some script loads, lower case.
    pub fn module_names(&self) -> BTreeSet<&str> {
        self.scripts
            .iter()
            .flat_map(|s| s.loads.iter().map(String::as_str))
            .collect()
    }
}

/// Whether `text` names `module` as a whole word (`mrxtaskjob` is not
/// named by `mrxtaskjobverifyset`).
fn mentions(text: &str, module: &str) -> bool {
    text.match_indices(module).any(|(at, _)| {
        let word = |c: Option<char>| c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        !word(text[..at].chars().next_back()) && !word(text[at + module.len()..].chars().next())
    })
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
        assert!(mentions("mrxtaskjob: started", "mrxtaskjob"));
        assert!(!mentions("mrxtaskjobverifyset", "mrxtaskjob"));
        assert_eq!(
            lib.module_names().into_iter().collect::<Vec<_>>(),
            ["task", "util"]
        );
    }
}
