use core::fmt;
use core::marker::PhantomData;

/// A 32-bit pointer as stored inside the original executable's structures.
///
/// This is an address, not a Rust reference: it carries no lifetime and is
/// never dereferenced implicitly. Use [`Ptr32::as_ptr`] inside the game
/// process (32-bit target only) when the pointee is known to be live.
#[repr(transparent)]
pub struct Ptr32<T> {
    addr: u32,
    _marker: PhantomData<*mut T>,
}

impl<T> Ptr32<T> {
    pub const NULL: Self = Self::from_addr(0);

    pub const fn from_addr(addr: u32) -> Self {
        Self {
            addr,
            _marker: PhantomData,
        }
    }

    pub const fn addr(self) -> u32 {
        self.addr
    }

    pub const fn is_null(self) -> bool {
        self.addr == 0
    }

    pub const fn cast<U>(self) -> Ptr32<U> {
        Ptr32::from_addr(self.addr)
    }

    /// Converts to a native pointer. Only available where native pointers are
    /// 32 bits wide, i.e. inside the original process.
    #[cfg(target_pointer_width = "32")]
    pub fn as_ptr(self) -> *mut T {
        self.addr as usize as *mut T
    }

    #[cfg(target_pointer_width = "32")]
    pub fn from_ptr(ptr: *mut T) -> Self {
        Self::from_addr(ptr as usize as u32)
    }
}

// Manual impls: derives would put bounds on `T`, which the pointee never needs.
impl<T> Clone for Ptr32<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Ptr32<T> {}

impl<T> PartialEq for Ptr32<T> {
    fn eq(&self, other: &Self) -> bool {
        self.addr == other.addr
    }
}

impl<T> Eq for Ptr32<T> {}

impl<T> Default for Ptr32<T> {
    fn default() -> Self {
        Self::NULL
    }
}

impl<T> fmt::Debug for Ptr32<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Ptr32({:#010x})", self.addr)
    }
}

// SAFETY: `Ptr32<T>` is `repr(transparent)` over `u32` (the marker is a ZST),
// so every bit pattern is valid and there is no padding.
unsafe impl<T: 'static> bytemuck::Zeroable for Ptr32<T> {}
// SAFETY: as above.
unsafe impl<T: 'static> bytemuck::Pod for Ptr32<T> {}

const _: () = assert!(core::mem::size_of::<Ptr32<u64>>() == 4);
const _: () = assert!(core::mem::align_of::<Ptr32<u64>>() == 4);
