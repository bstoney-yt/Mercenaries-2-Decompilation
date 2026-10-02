//! Reconstructed engine classes.
//!
//! Each class gets a raw `#[repr(C)]` mirror (with its vtable as an explicit
//! struct of function pointers) in a `raw` module, and a safe Rust type or
//! trait on top. Rust `dyn Trait` vtables are not ABI-compatible with MSVC C++
//! vtables, so the two are never mixed.
