//! Bounds-checked little-endian reads from a byte slice.

/// The read ran past the end of the buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OutOfBounds {
    /// Offset of the attempted read.
    pub offset: usize,
    /// Bytes requested.
    pub len: usize,
}

/// A cursor over a byte slice. Every read checks bounds and never panics.
#[derive(Debug, Clone)]
pub(crate) struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Starts reading `buf` at `pos`.
    pub fn at(buf: &'a [u8], pos: usize) -> Self {
        Self { buf, pos }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn take(&mut self, len: usize) -> Result<&'a [u8], OutOfBounds> {
        let oob = OutOfBounds {
            offset: self.pos,
            len,
        };
        let end = self.pos.checked_add(len).ok_or(oob)?;
        let bytes = self.buf.get(self.pos..end).ok_or(oob)?;
        self.pos = end;
        Ok(bytes)
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], OutOfBounds> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub fn u16(&mut self) -> Result<u16, OutOfBounds> {
        self.array().map(u16::from_le_bytes)
    }

    pub fn u32(&mut self) -> Result<u32, OutOfBounds> {
        self.array().map(u32::from_le_bytes)
    }

    /// Reads a NUL-terminated string and skips the terminator.
    pub fn cstr(&mut self) -> Result<&'a [u8], OutOfBounds> {
        let rest = self.buf.get(self.pos..).unwrap_or_default();
        let len = rest.iter().position(|&b| b == 0).ok_or(OutOfBounds {
            offset: self.pos,
            len: rest.len() + 1,
        })?;
        let s = self.take(len)?;
        self.pos += 1;
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian_and_stops_at_end() {
        let mut r = Reader::at(&[1, 2, 3, 4, 5, 6], 0);
        assert_eq!(r.u32(), Ok(0x0403_0201));
        assert_eq!(r.u16(), Ok(0x0605));
        assert_eq!(r.u16(), Err(OutOfBounds { offset: 6, len: 2 }));
    }

    #[test]
    fn cstr_requires_terminator() {
        let mut r = Reader::at(b"ab\0cd", 0);
        assert_eq!(r.cstr(), Ok(&b"ab"[..]));
        assert_eq!(r.pos(), 3);
        assert!(r.cstr().is_err());
    }

    #[test]
    fn huge_take_does_not_overflow() {
        let mut r = Reader::at(&[0; 4], 2);
        assert!(r.take(usize::MAX).is_err());
    }
}
