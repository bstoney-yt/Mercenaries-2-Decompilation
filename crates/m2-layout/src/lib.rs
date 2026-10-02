//! Binary-compatible primitives for structures recovered from the original
//! 32-bit executable.
//!
//! Raw structs mirror the original memory layout exactly and live behind
//! `#[repr(C)]`. Pointers inside them are stored as [`Ptr32`] rather than
//! native pointers so that `size_of`/`offset_of!` checks give the same answer
//! on a 64-bit development host as on the 32-bit game process. Idiomatic types
//! in higher crates convert from these at the boundary.

#![no_std]

mod addr;
mod ptr32;

pub use addr::{IMAGE_BASE, Rva, Va};
pub use ptr32::Ptr32;

/// Asserts the size of a raw struct and the offsets of its fields at compile
/// time.
///
/// ```
/// use m2_layout::{assert_layout, Ptr32};
///
/// #[repr(C)]
/// struct Node {
///     next: Ptr32<Node>,
///     flags: u16,
///     kind: u16,
///     value: f32,
/// }
///
/// assert_layout!(Node, size = 0xC, {
///     next: 0x0,
///     flags: 0x4,
///     kind: 0x6,
///     value: 0x8,
/// });
/// ```
#[macro_export]
macro_rules! assert_layout {
    ($ty:ty, size = $size:expr $(, { $($field:ident : $offset:expr),* $(,)? })? $(,)?) => {
        const _: () = {
            assert!(::core::mem::size_of::<$ty>() == $size, concat!("size of ", stringify!($ty)));
            $($(
                assert!(
                    ::core::mem::offset_of!($ty, $field) == $offset,
                    concat!("offset of ", stringify!($ty), "::", stringify!($field)),
                );
            )*)?
        };
    };
}
