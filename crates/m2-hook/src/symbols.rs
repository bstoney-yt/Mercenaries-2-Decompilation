use std::collections::HashMap;
use std::path::Path;

use m2_layout::Rva;
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallingConvention {
    Cdecl,
    Stdcall,
    Thiscall,
    Fastcall,
    /// Non-standard register usage; describe it in `notes`.
    Custom,
}

/// How far along a function is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Located, purpose not yet understood.
    Found,
    /// Purpose and signature understood.
    Named,
    /// Rust translation exists.
    Translated,
    /// Rust translation matches the original under differential tests.
    Verified,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionSymbol {
    pub name: String,
    /// Offset from the image base, written as a hex integer (`rva = 0x1234`).
    pub rva: u32,
    pub convention: CallingConvention,
    /// C-style signature, e.g. `float __thiscall Vector3::Length(Vector3 *this)`.
    pub signature: String,
    pub status: Status,
    /// Path of the Rust translation, e.g. `m2_math::vec3_length`.
    #[serde(default)]
    pub rust: Option<String>,
    #[serde(default)]
    pub notes: Option<String>,
}

impl FunctionSymbol {
    pub fn rva(&self) -> Rva {
        Rva(self.rva)
    }
}

/// One `symbols/*.toml` file.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolFile {
    #[serde(default, rename = "function")]
    pub functions: Vec<FunctionSymbol>,
}

#[derive(Debug, thiserror::Error)]
pub enum SymbolError {
    #[error("{path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
    #[error("{path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("duplicate function name `{name}` ({first} and {second})")]
    DuplicateName {
        name: String,
        first: String,
        second: String,
    },
    #[error("duplicate rva {rva:#x} for `{first}` and `{second}`")]
    DuplicateRva {
        rva: u32,
        first: String,
        second: String,
    },
    #[error("`{name}` is {status:?} but has no `rust` path")]
    MissingRustPath { name: String, status: Status },
}

/// All symbol files merged and checked for consistency.
#[derive(Debug, Default)]
pub struct SymbolTable {
    functions: Vec<(String, FunctionSymbol)>,
}

impl SymbolTable {
    /// Loads every `*.toml` file in `dir`.
    pub fn load_dir(dir: &Path) -> Result<Self, SymbolError> {
        let io = |source| SymbolError::Io {
            path: dir.display().to_string(),
            source,
        };
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(dir).map_err(io)? {
            let path = entry.map_err(io)?.path();
            if path.extension().is_some_and(|e| e == "toml") {
                paths.push(path);
            }
        }
        paths.sort();

        let mut files = Vec::new();
        for path in paths {
            let display = path.display().to_string();
            let text = std::fs::read_to_string(&path).map_err(|source| SymbolError::Io {
                path: display.clone(),
                source,
            })?;
            let file = toml::from_str(&text).map_err(|source| SymbolError::Parse {
                path: display.clone(),
                source,
            })?;
            files.push((display, file));
        }
        Self::from_files(files)
    }

    /// Merges parsed files, keyed by their path for error messages.
    pub fn from_files(files: Vec<(String, SymbolFile)>) -> Result<Self, SymbolError> {
        let mut table = Self::default();
        let mut by_name: HashMap<String, String> = HashMap::new();
        let mut by_rva: HashMap<u32, String> = HashMap::new();

        for (path, file) in files {
            for function in file.functions {
                if function.status >= Status::Translated && function.rust.is_none() {
                    return Err(SymbolError::MissingRustPath {
                        name: function.name,
                        status: function.status,
                    });
                }
                if let Some(first) = by_name.insert(function.name.clone(), path.clone()) {
                    return Err(SymbolError::DuplicateName {
                        name: function.name,
                        first,
                        second: path,
                    });
                }
                if let Some(first) = by_rva.insert(function.rva, function.name.clone()) {
                    return Err(SymbolError::DuplicateRva {
                        rva: function.rva,
                        first,
                        second: function.name,
                    });
                }
                table.functions.push((path.clone(), function));
            }
        }
        table.functions.sort_by_key(|(_, f)| f.rva);
        Ok(table)
    }

    pub fn functions(&self) -> impl Iterator<Item = &FunctionSymbol> {
        self.functions.iter().map(|(_, f)| f)
    }

    pub fn by_name(&self, name: &str) -> Option<&FunctionSymbol> {
        self.functions().find(|f| f.name == name)
    }

    pub fn len(&self) -> usize {
        self.functions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.functions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> SymbolFile {
        toml::from_str(text).unwrap()
    }

    const SAMPLE: &str = r#"
        [[function]]
        name = "Vector3::Length"
        rva = 0x1000
        convention = "thiscall"
        signature = "float __thiscall Vector3::Length(Vector3 *this)"
        status = "translated"
        rust = "m2_math::RawVec3::length"
    "#;

    #[test]
    fn parses_sample() {
        let file = parse(SAMPLE);
        assert_eq!(file.functions.len(), 1);
        assert_eq!(file.functions[0].rva(), Rva(0x1000));
        assert_eq!(file.functions[0].convention, CallingConvention::Thiscall);
    }

    #[test]
    fn rejects_duplicate_rva() {
        let mut other = parse(SAMPLE);
        other.functions[0].name = "Other".into();
        let err = SymbolTable::from_files(vec![("a".into(), parse(SAMPLE)), ("b".into(), other)])
            .unwrap_err();
        assert!(matches!(err, SymbolError::DuplicateRva { rva: 0x1000, .. }));
    }

    #[test]
    fn translated_needs_rust_path() {
        let mut file = parse(SAMPLE);
        file.functions[0].rust = None;
        let err = SymbolTable::from_files(vec![("a".into(), file)]).unwrap_err();
        assert!(matches!(err, SymbolError::MissingRustPath { .. }));
    }

    #[test]
    fn repo_symbols_are_consistent() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../symbols");
        SymbolTable::load_dir(&dir).unwrap();
    }
}
