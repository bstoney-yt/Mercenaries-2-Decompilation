//! Memory management.
//!
//! Translated allocator, pool and container functions land here. The goal is
//! to replace the original manual lifetimes with owned types (`Box`, `Vec`,
//! arenas, generational handles) while raw structs still see the addresses
//! they expect during the hybrid phase.
