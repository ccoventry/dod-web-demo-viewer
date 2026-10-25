//! Minimal little-endian byte writer.
//!
//! `dem::byte_writer` is a private module, so this duplicates the few methods
//! we need rather than widening the fork's public surface.

pub struct ByteWriter {
    data: Vec<u8>,
}

impl ByteWriter {
    pub fn new() -> Self {
        Self { data: Vec::new() }
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            data: Vec::with_capacity(n),
        }
    }

    #[inline]
    pub fn offset(&self) -> usize {
        self.data.len()
    }

    #[inline]
    pub fn u8(&mut self, v: u8) {
        self.data.push(v);
    }

    #[inline]
    pub fn i32(&mut self, v: i32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn u16(&mut self, v: u16) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn u32(&mut self, v: u32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn f32(&mut self, v: f32) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn f64(&mut self, v: f64) {
        self.data.extend_from_slice(&v.to_le_bytes());
    }

    #[inline]
    pub fn bytes(&mut self, v: &[u8]) {
        self.data.extend_from_slice(v);
    }

    /// Write `src` into a fixed-width NUL-padded field, always NUL-terminated.
    pub fn fixed_str(&mut self, src: &[u8], width: usize) {
        let end = src.iter().position(|&b| b == 0).unwrap_or(src.len());
        let take = end.min(width.saturating_sub(1));
        self.data.extend_from_slice(&src[..take]);
        self.data.extend(std::iter::repeat(0u8).take(width - take));
    }

    /// Backpatch a previously reserved i32 slot.
    pub fn patch_i32(&mut self, at: usize, v: i32) {
        self.data[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    pub fn into_vec(self) -> Vec<u8> {
        self.data
    }
}

impl Default for ByteWriter {
    fn default() -> Self {
        Self::new()
    }
}
