use core::fmt;

/// Default preferred image base for 32-bit Windows executables.
///
/// Confirm against the retail exe's PE header; symbol tables store RVAs so a
/// different base only changes this constant.
pub const IMAGE_BASE: u32 = 0x0040_0000;

/// Relative virtual address: an offset from the image base. Symbol tables
/// store these so they stay valid if the image is relocated.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Rva(pub u32);

/// Absolute virtual address inside the running 32-bit process.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Va(pub u32);

impl Rva {
    pub const fn to_va(self, image_base: u32) -> Va {
        Va(image_base.wrapping_add(self.0))
    }
}

impl Va {
    pub const fn to_rva(self, image_base: u32) -> Rva {
        Rva(self.0.wrapping_sub(image_base))
    }
}

impl fmt::Debug for Rva {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Rva({:#010x})", self.0)
    }
}

impl fmt::Debug for Va {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Va({:#010x})", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rva_va_round_trip() {
        let rva = Rva(0x1234);
        let va = rva.to_va(IMAGE_BASE);
        assert_eq!(va, Va(0x0040_1234));
        assert_eq!(va.to_rva(IMAGE_BASE), rva);
    }
}
