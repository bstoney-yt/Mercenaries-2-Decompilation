//! Symbol tables and the per-function switch between original and Rust code.
//!
//! Every function we identify gets an entry in `symbols/*.toml`. Once it has a
//! Rust translation, the loader can route calls to either implementation, so
//! translations can be swapped in one at a time and compared against the
//! original. The detour backend that patches the executable is added once the
//! first translated function is ready to hook.

pub mod symbols;

pub use symbols::{CallingConvention, FunctionSymbol, Status, SymbolFile, SymbolTable};

/// Which implementation a hooked function routes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Implementation {
    #[default]
    Original,
    Rust,
}
