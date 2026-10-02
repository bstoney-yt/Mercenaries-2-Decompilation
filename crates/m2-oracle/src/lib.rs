//! Differential testing.
//!
//! Runs an original function and its Rust translation on the same inputs and
//! compares results, with per-function float tolerances (the original may use
//! x87 80-bit intermediates; Rust on i686 uses SSE2). Needs the user's own
//! executable, so these tests are opt-in and never run in CI.
