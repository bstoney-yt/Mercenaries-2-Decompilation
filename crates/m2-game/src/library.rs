//! The game's scripts as modules: which script `import("MrxUtil")` means.
//!
//! Scripts load each other by module name (`import`, `inherit`,
//! `dynamic_import`), and the archives key scripts by [`name_hash`] of that
//! name, so a module is the script with its name's hash. For a name no
//! script has (a test module, or a name the hash somehow misses), each
//! script runs in its own environment and defines its module's functions
//! there as globals, and other scripts call them as
//! `MrxUtil.GetRandomTableElement`; so the module is then the script that
//! defines most of the members other scripts use on that name, among the
//! scripts that don't themselves load it.

use std::collections::{BTreeMap, BTreeSet};

use m2_assets::lua::{Chunk, NameUse};
use m2_assets::name_hash;

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
    /// Lower-case module name to the script it is known to be, whatever
    /// its hash and the member match say.
    pinned: BTreeMap<String, u32>,
}

/// Modules the member match got wrong before the name hash was known, by
/// the script they are, found then by what each script inherits and
/// defines; now a check of the hash.
///
/// The task classes were the trouble: subclasses call so many base-class
/// members through `MrxTaskMission` and `MrxTaskObjective` that the match
/// gave those names the base class and contracts.
pub const KNOWN_MODULES: [(&str, u32); 16] = [
    ("MrxBoatDelivery", 0x5b6a_0cf6),
    ("MrxClusterBomb", 0xeaf4_7b4a),
    ("MrxCopterDrop", 0x3cca_21b4),
    ("MrxCrateDelivery", 0x3534_ce91),
    ("MrxDaisyCutter", 0xd913_5f63),
    ("MrxMOAB", 0x0f1b_9dfd),
    ("MrxSupportDelivery", 0x0eac_5ae9),
    ("MrxSupportDesignatorFlare", 0x27c3_28f1),
    ("MrxSupportDesignatorSmoke", 0xecb5_bbea),
    ("MrxTask", 0xd302_a66f),
    ("MrxTaskMission", 0xcfcd_908f),
    ("MrxTaskObjective", 0x517a_1910),
    ("MrxTaskObjectiveDeliver", 0x72c1_306f),
    ("MrxTaskObjectiveDestroy", 0xf554_9eba),
    ("MrxTaskObjectiveVerify", 0x3cec_2cbb),
    ("MrxTaskRace", 0x2595_ba98),
];

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

    /// Makes module `name` script `hash`, whatever the member match says.
    /// Call before the first [`Library::resolve`].
    pub fn pin(&mut self, name: &str, hash: u32) {
        self.pinned.insert(name.to_ascii_lowercase(), hash);
    }

    pub fn scripts(&self) -> &[ScriptInfo] {
        &self.scripts
    }

    pub fn by_hash(&self, hash: u32) -> Option<&ScriptInfo> {
        self.scripts.iter().find(|s| s.hash == hash)
    }

    /// The script that is module `name` (any case), with how many of the
    /// members used on it it defines, out of how many. `None` when no
    /// script has the name's hash and none defines any of them (or nothing
    /// is used on it).
    ///
    /// Without a script of the name's hash, every module is scored against
    /// every script that doesn't load it:
    /// one point per member used on the module that the script defines,
    /// ten more when one of its strings starts with the module's name and
    /// a colon (scripts prefix their debug messages with their own name,
    /// `"MrxTaskContract:OnPlayerJoined"`), else two when a string names
    /// it. The names settle classes whose few used members, such as
    /// `Activated`, many scripts define.
    /// The best pairs are taken first and a script is one module only.
    ///
    /// A module the member match leaves out (a mission such as
    /// `VzaCon001`, loaded by a name from data) is the script not already
    /// some module's that names it most often in its strings.
    pub fn resolve(&self, name: &str) -> Option<(&ScriptInfo, usize, usize)> {
        let key = name.to_ascii_lowercase();
        let assigned = self.assigned.get_or_init(|| self.assign());
        if let Some(&(index, hits)) = assigned.get(&key) {
            let used = self.used.get(&key).map_or(0, BTreeSet::len);
            return Some((&self.scripts[index], hits, used));
        }
        if let Some(s) = self.by_hash(name_hash(&key)) {
            return Some((s, 0, 0));
        }
        let taken: BTreeSet<usize> = assigned.values().map(|&(i, _)| i).collect();
        self.scripts
            .iter()
            .enumerate()
            // A script never is a module it loads (a subclass inherits
            // its base).
            .filter(|(i, s)| !taken.contains(i) && !s.loads.contains(&key))
            .map(|(_, s)| (s.strings.iter().filter(|t| mentions(t, &key)).count(), s))
            .filter(|&(n, _)| n > 0)
            .max_by(|a, b| {
                a.0.cmp(&b.0)
                    .then(b.1.defines.len().cmp(&a.1.defines.len()))
            })
            .map(|(_, s)| (s, 0, 0))
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
                // Messages that start "MrxTaskContract:..." name their own
                // script; a mention elsewhere in a string is weaker.
                let own = s.strings.iter().any(|t| {
                    t.strip_prefix(module.as_str())
                        .is_some_and(|rest| rest.starts_with(':'))
                });
                let named = s.strings.iter().any(|t| mentions(t, module));
                let bonus = if own {
                    10
                } else if named {
                    2
                } else {
                    0
                };
                pairs.push((hits + bonus, hits, module, i));
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
        let by_name: Vec<(String, u32)> = self
            .module_names()
            .into_iter()
            .map(|m| (m.to_owned(), name_hash(m)))
            .collect();
        for (module, &hash) in self
            .pinned
            .iter()
            .chain(by_name.iter().map(|(m, h)| (m, h)))
        {
            if out.contains_key(module) {
                continue;
            }
            if let Some(i) = self.scripts.iter().position(|s| s.hash == hash) {
                let hits = self.used.get(module).map_or(0, |used| {
                    used.iter()
                        .filter(|m| self.scripts[i].defines.contains(*m))
                        .count()
                });
                taken.insert(i);
                out.insert(module.clone(), (i, hits));
            }
        }
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
            // Defines only Activated, which the contract uses on Task.
            (5, include_bytes!("../tests/data/decoy.luac")),
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
        // The contract defines Activated too, but it inherits Task; the
        // decoy defines less else, but Task names itself.
        assert_eq!(lib.resolve("task").map(|(s, _, _)| s.hash), Some(2));
        assert!(lib.resolve("Nothing").is_none());
        assert!(mentions("mrxtaskjob: started", "mrxtaskjob"));
        assert!(!mentions("mrxtaskjobverifyset", "mrxtaskjob"));
        assert_eq!(
            lib.module_names().into_iter().collect::<Vec<_>>(),
            ["task", "util"]
        );
    }

    #[test]
    fn known_modules_have_their_names_hash() {
        for (name, hash) in KNOWN_MODULES {
            assert_eq!(name_hash(name), hash, "{name}");
        }
    }

    #[test]
    fn a_script_with_the_names_hash_is_the_module() {
        let mut lib = library();
        // The decoy defines only Activated, but it has the name's hash.
        let bytes = &include_bytes!("../tests/data/decoy.luac")[..];
        lib.add(name_hash("Task"), Chunk::parse(bytes).unwrap());
        assert_eq!(
            lib.resolve("task").map(|(s, h, _)| (s.hash, h)),
            Some((name_hash("Task"), 1))
        );
        // A name nothing loads is found by hash too.
        lib.add(name_hash("Lonely"), Chunk::parse(bytes).unwrap());
        assert_eq!(
            lib.resolve("Lonely").map(|(s, _, _)| s.hash),
            Some(name_hash("Lonely"))
        );
    }

    #[test]
    fn pins_win_over_the_match() {
        let mut lib = library();
        lib.pin("Task", 5);
        assert_eq!(
            lib.resolve("task").map(|(s, h, _)| (s.hash, h)),
            Some((5, 1))
        );
        // The pinned script is no other module's.
        assert_eq!(lib.resolve("Util").map(|(s, _, _)| s.hash), Some(1));
    }
}
