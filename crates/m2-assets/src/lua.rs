//! Compiled Lua scripts: mission logic, the front end's widgets and a few
//! tables.
//!
//! A script asset (type hash `42498680`, 645 in `vz.wad`, 28 in
//! `shell.wad`) is a `UCFX` container of `INFO` and `BINN`; `BINN` holds a
//! precompiled Lua chunk as written by `luac`. This module reads such
//! chunks (Lua 5.0 and 5.1 layouts, either byte order) into their function
//! tree: code, constants and nested functions, with the debug information
//! when present. It does not run them.
//!
//! [`Function::names`] lists the global names a chunk reads or writes and
//! the constant field names it indexes with, which together are the engine
//! functions and tables a script expects to find.
//!
//! # Provenance
//!
//! Implemented from the published Lua 5.0 and 5.1 sources' description of
//! the precompiled chunk format (`lundump.c`, `lopcodes.h`; MIT licence),
//! and checked against chunks this repository compiles from its own test
//! source. Which version the game uses is read from each chunk's header.

use crate::bytes::{OutOfBounds, Reader};
use crate::ucfx::{Ucfx, UcfxError};

/// Asset type hash of compiled scripts.
pub const SCRIPT_TYPE: u32 = 0x4249_8680;

/// Errors from reading a chunk.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum LuaError {
    #[error("container: {0}")]
    Ucfx(#[from] UcfxError),
    #[error("missing BINN section")]
    MissingBody,
    #[error("not a Lua chunk")]
    NotLua,
    #[error("Lua version {0:#04x} is not read")]
    Version(u8),
    #[error(
        "unsupported sizes: int {int}, size_t {size_t}, instruction {instruction}, number {number}"
    )]
    Sizes {
        int: u8,
        size_t: u8,
        instruction: u8,
        number: u8,
    },
    #[error("truncated: needed {len} bytes at {offset:#x}")]
    Truncated { offset: usize, len: usize },
    #[error("constant of unknown type {0}")]
    ConstantType(u8),
    #[error("functions nested too deep")]
    TooDeep,
}

impl From<OutOfBounds> for LuaError {
    fn from(e: OutOfBounds) -> Self {
        Self::Truncated {
            offset: e.offset,
            len: e.len,
        }
    }
}

/// The two chunk layouts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Lua50,
    Lua51,
}

/// A constant.
#[derive(Clone, Debug, PartialEq)]
pub enum Constant {
    Nil,
    Bool(bool),
    Number(f64),
    String(String),
}

/// One function prototype.
#[derive(Clone, Debug, PartialEq)]
pub struct Function {
    pub source: Option<String>,
    pub line_defined: i32,
    pub upvalue_count: u8,
    pub param_count: u8,
    pub is_vararg: u8,
    pub max_stack: u8,
    pub code: Vec<u32>,
    pub constants: Vec<Constant>,
    pub functions: Vec<Function>,
    /// Line per instruction (debug information; may be empty).
    pub lines: Vec<i32>,
    /// Local variable names (debug information; may be empty).
    pub locals: Vec<String>,
    pub upvalues: Vec<String>,
}

/// A whole chunk: its layout and main function.
#[derive(Clone, Debug, PartialEq)]
pub struct Chunk {
    pub version: Version,
    pub big_endian: bool,
    pub main: Function,
}

/// How names are used, from [`Function::names`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum NameUse {
    /// Read as a global (`GETGLOBAL`).
    GetGlobal,
    /// Assigned as a global (`SETGLOBAL`): functions the script defines.
    SetGlobal,
    /// A constant key indexed or called as a method (`GETTABLE`, `SELF`).
    Field,
    /// A constant key read from a global table: `"Global.key"`.
    Member,
}

struct Layout {
    version: Version,
    big: bool,
    size_t: u8,
}

impl Chunk {
    /// Reads a script asset: the chunk in its `BINN` section.
    pub fn from_asset(asset: &[u8]) -> Result<Self, LuaError> {
        let ucfx = Ucfx::parse(asset)?;
        let body = ucfx
            .sections()
            .iter()
            .find(|s| &s.fourcc == b"BINN")
            .and_then(|s| s.data)
            .ok_or(LuaError::MissingBody)?;
        Self::parse(body)
    }

    /// Reads a precompiled chunk.
    pub fn parse(data: &[u8]) -> Result<Self, LuaError> {
        if !data.starts_with(b"\x1bLua") {
            return Err(LuaError::NotLua);
        }
        let mut r = Reader::at(data, 0);
        r.take(4)?;
        let version = match r.u8()? {
            0x50 => Version::Lua50,
            0x51 => Version::Lua51,
            v => return Err(LuaError::Version(v)),
        };
        let (big, int, size_t, instruction, number) = match version {
            Version::Lua51 => {
                let _format = r.u8()?;
                let little = r.u8()?;
                let (int, size_t, instruction, number) = (r.u8()?, r.u8()?, r.u8()?, r.u8()?);
                let _integral = r.u8()?;
                (little == 0, int, size_t, instruction, number)
            }
            Version::Lua50 => {
                let little = r.u8()?;
                let (int, size_t, instruction) = (r.u8()?, r.u8()?, r.u8()?);
                // Opcode field widths (SIZE_OP, A, B, C): the standard
                // 6, 8, 9, 9 is assumed by the decoder below.
                r.take(4)?;
                let number = r.u8()?;
                // A test number (3.14159265358979323846e7).
                r.take(usize::from(number))?;
                (little == 0, int, size_t, instruction, number)
            }
        };
        if int != 4
            || !(size_t == 4 || size_t == 8)
            || instruction != 4
            || !(number == 4 || number == 8)
        {
            return Err(LuaError::Sizes {
                int,
                size_t,
                instruction,
                number,
            });
        }
        let layout = Layout {
            version,
            big,
            size_t,
        };
        let mut p = Parser {
            r,
            layout: &layout,
            number,
        };
        let main = p.function(0)?;
        Ok(Self {
            version,
            big_endian: big,
            main,
        })
    }
}

struct Parser<'a, 'l> {
    r: Reader<'a>,
    layout: &'l Layout,
    number: u8,
}

impl Parser<'_, '_> {
    fn int(&mut self) -> Result<i32, LuaError> {
        let b = self.r.take(4)?;
        let b = [b[0], b[1], b[2], b[3]];
        Ok(if self.layout.big {
            i32::from_be_bytes(b)
        } else {
            i32::from_le_bytes(b)
        })
    }

    fn count(&mut self) -> Result<usize, LuaError> {
        let n = self.int()?;
        // Every counted item takes at least one byte.
        let n = usize::try_from(n).map_err(|_| LuaError::Truncated {
            offset: self.r.pos(),
            len: usize::MAX,
        })?;
        if n > self.r.remaining() {
            return Err(LuaError::Truncated {
                offset: self.r.pos(),
                len: n,
            });
        }
        Ok(n)
    }

    fn size(&mut self) -> Result<usize, LuaError> {
        let b = self.r.take(usize::from(self.layout.size_t))?;
        let mut v: u64 = 0;
        for (i, &x) in b.iter().enumerate() {
            let shift = if self.layout.big {
                8 * (b.len() - 1 - i)
            } else {
                8 * i
            };
            v |= u64::from(x) << shift;
        }
        usize::try_from(v).map_err(|_| LuaError::Truncated {
            offset: self.r.pos(),
            len: usize::MAX,
        })
    }

    /// A string: size including the trailing NUL, 0 for none.
    fn string(&mut self) -> Result<Option<String>, LuaError> {
        let n = self.size()?;
        if n == 0 {
            return Ok(None);
        }
        let b = self.r.take(n)?;
        let text = &b[..n - 1];
        Ok(Some(String::from_utf8_lossy(text).into_owned()))
    }

    fn number(&mut self) -> Result<f64, LuaError> {
        let b = self.r.take(usize::from(self.number))?;
        Ok(match (self.number, self.layout.big) {
            (8, false) => f64::from_le_bytes(b.try_into().expect("8 bytes")),
            (8, true) => f64::from_be_bytes(b.try_into().expect("8 bytes")),
            (_, false) => f64::from(f32::from_le_bytes(b.try_into().expect("4 bytes"))),
            (_, true) => f64::from(f32::from_be_bytes(b.try_into().expect("4 bytes"))),
        })
    }

    fn code(&mut self) -> Result<Vec<u32>, LuaError> {
        let n = self.count()?;
        (0..n).map(|_| Ok(self.int()? as u32)).collect()
    }

    fn constants(&mut self) -> Result<Vec<Constant>, LuaError> {
        let n = self.count()?;
        (0..n)
            .map(|_| {
                Ok(match self.r.u8()? {
                    0 => Constant::Nil,
                    1 => Constant::Bool(self.r.u8()? != 0),
                    3 => Constant::Number(self.number()?),
                    4 => Constant::String(self.string()?.unwrap_or_default()),
                    t => return Err(LuaError::ConstantType(t)),
                })
            })
            .collect()
    }

    fn functions(&mut self, depth: usize) -> Result<Vec<Function>, LuaError> {
        let n = self.count()?;
        (0..n).map(|_| self.function(depth + 1)).collect()
    }

    fn lines(&mut self) -> Result<Vec<i32>, LuaError> {
        let n = self.count()?;
        (0..n).map(|_| self.int()).collect()
    }

    fn locals(&mut self) -> Result<Vec<String>, LuaError> {
        let n = self.count()?;
        (0..n)
            .map(|_| {
                let name = self.string()?.unwrap_or_default();
                self.int()?;
                self.int()?;
                Ok(name)
            })
            .collect()
    }

    fn upvalues(&mut self) -> Result<Vec<String>, LuaError> {
        let n = self.count()?;
        (0..n)
            .map(|_| Ok(self.string()?.unwrap_or_default()))
            .collect()
    }

    fn function(&mut self, depth: usize) -> Result<Function, LuaError> {
        if depth > 200 {
            return Err(LuaError::TooDeep);
        }
        let source = self.string()?;
        let line_defined = self.int()?;
        if self.layout.version == Version::Lua51 {
            let _last_line = self.int()?;
        }
        let upvalue_count = self.r.u8()?;
        let param_count = self.r.u8()?;
        let is_vararg = self.r.u8()?;
        let max_stack = self.r.u8()?;
        let mut f = Function {
            source,
            line_defined,
            upvalue_count,
            param_count,
            is_vararg,
            max_stack,
            code: Vec::new(),
            constants: Vec::new(),
            functions: Vec::new(),
            lines: Vec::new(),
            locals: Vec::new(),
            upvalues: Vec::new(),
        };
        match self.layout.version {
            Version::Lua51 => {
                f.code = self.code()?;
                f.constants = self.constants()?;
                f.functions = self.functions(depth)?;
                f.lines = self.lines()?;
                f.locals = self.locals()?;
                f.upvalues = self.upvalues()?;
            }
            Version::Lua50 => {
                f.lines = self.lines()?;
                f.locals = self.locals()?;
                f.upvalues = self.upvalues()?;
                f.constants = self.constants()?;
                f.functions = self.functions(depth)?;
                f.code = self.code()?;
            }
        }
        Ok(f)
    }
}

impl Function {
    /// This function and every nested one, depth first.
    pub fn all(&self) -> Vec<&Function> {
        let mut out = vec![self];
        for f in &self.functions {
            out.extend(f.all());
        }
        out
    }

    fn constant_name(&self, index: usize) -> Option<&str> {
        match self.constants.get(index) {
            Some(Constant::String(s)) => Some(s),
            _ => None,
        }
    }

    /// The names this function and its nested functions use: globals read
    /// and written, and constant keys indexed or called as methods. A key
    /// read from a register just loaded from a global comes back as
    /// [`NameUse::Member`] `"Global.key"`, so library calls such as
    /// `Objective.Add` read whole.
    pub fn names(&self, version: Version) -> Vec<(NameUse, String)> {
        let mut out = Vec::new();
        for f in self.all() {
            // Registers holding a global, from the last GETGLOBAL into them.
            let mut held: Vec<Option<&str>> = vec![None; 256];
            for &i in &f.code {
                let (op, a, b, bx, rk) = match version {
                    // B:9 C:9 A:8 above a 6-bit opcode; RK constants have
                    // bit 8 set.
                    Version::Lua51 => {
                        let c = (i >> 14) & 0x1ff;
                        let k = (c & 0x100 != 0).then_some((c & 0xff) as usize);
                        (
                            i & 0x3f,
                            (i >> 6) & 0xff,
                            (i >> 23) & 0x1ff,
                            (i >> 14) as usize,
                            k,
                        )
                    }
                    // A:8 B:9 C:9 above a 6-bit opcode; RK constants are
                    // at 250 and up.
                    Version::Lua50 => {
                        let c = (i >> 6) & 0x1ff;
                        let k = (c >= 250).then(|| (c - 250) as usize);
                        (
                            i & 0x3f,
                            i >> 24,
                            (i >> 15) & 0x1ff,
                            ((i >> 6) & 0x3ffff) as usize,
                            k,
                        )
                    }
                };
                let receiver = held.get(b as usize).copied().flatten();
                // Opcodes 5, 6, 7 and 11 are GETGLOBAL, GETTABLE, SETGLOBAL
                // and SELF in both versions.
                match op {
                    5 => {
                        if let Some(n) = f.constant_name(bx) {
                            out.push((NameUse::GetGlobal, n.to_owned()));
                        }
                    }
                    7 => {
                        if let Some(n) = f.constant_name(bx) {
                            out.push((NameUse::SetGlobal, n.to_owned()));
                        }
                    }
                    6 | 11 => {
                        if let Some(n) = rk.and_then(|k| f.constant_name(k)) {
                            out.push(match receiver {
                                Some(g) if op == 6 => (NameUse::Member, format!("{g}.{n}")),
                                _ => (NameUse::Field, n.to_owned()),
                            });
                        }
                    }
                    _ => {}
                }
                // Most instructions write register A; SETGLOBAL only reads it.
                if op != 7
                    && let Some(r) = held.get_mut(a as usize)
                {
                    *r = if op == 5 { f.constant_name(bx) } else { None };
                }
            }
        }
        out
    }

    /// Every string constant in this function and its nested ones.
    pub fn strings(&self) -> Vec<&str> {
        self.all()
            .into_iter()
            .flat_map(|f| f.constants.iter())
            .filter_map(|c| match c {
                Constant::String(s) => Some(s.as_str()),
                _ => None,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `luac5.1 -s` of:
    ///
    /// ```lua
    /// Mission = {}
    /// function Mission.Start(m)
    ///   Objective_Add("Destroy the depot", 3)
    ///   Hud.Show("timer")
    ///   m:SetTimer(30.5)
    /// end
    /// ```
    const LUA51: &[u8] = include_bytes!("../tests/data/mission51.luac");

    #[test]
    fn reads_a_lua51_chunk() {
        let chunk = Chunk::parse(LUA51).unwrap();
        assert_eq!(chunk.version, Version::Lua51);
        assert!(!chunk.big_endian);
        assert_eq!(chunk.main.functions.len(), 1);
        let start = &chunk.main.functions[0];
        assert_eq!(start.param_count, 1);
        assert!(start.strings().contains(&"Destroy the depot"));
        assert!(start.constants.contains(&Constant::Number(30.5)));
        let names = chunk.main.names(chunk.version);
        for want in [
            (NameUse::SetGlobal, "Mission"),
            (NameUse::GetGlobal, "Mission"),
            (NameUse::GetGlobal, "Objective_Add"),
            (NameUse::Member, "Hud.Show"),
            (NameUse::Field, "SetTimer"),
        ] {
            assert!(
                names.iter().any(|(u, n)| *u == want.0 && n == want.1),
                "{want:?} in {names:?}"
            );
        }
    }

    #[test]
    fn rejects_other_data() {
        assert_eq!(Chunk::parse(b"nope"), Err(LuaError::NotLua));
        assert!(matches!(
            Chunk::parse(b"\x1bLua\x52"),
            Err(LuaError::Version(0x52))
        ));
        assert!(Chunk::parse(&LUA51[..40]).is_err());
    }
}
