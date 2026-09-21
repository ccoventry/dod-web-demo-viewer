

use bitvec::{field::BitField, order::Lsb0, slice::BitSlice as _BitSlice};

use self::types::BitVec;

use super::*;

pub type BitSlice = _BitSlice<u8, Lsb0>;

// Wraps bytes into bits because doing this with nom is a very bad idea.
pub struct BitReader {
    pub bytes: BitVec,
    // Bit offset, starting from starting of `bytes`.
    offset: usize,
    // Set once any read has run past the end of `bytes`. See `is_bad_read`.
    bad_read: bool,
}

impl BitReader {
    pub fn new(bytes: &[u8]) -> Self {
        BitReader {
            bytes: BitVec::from_slice(bytes),
            offset: 0,
            bad_read: false,
        }
    }

    /// Whether any read so far ran past the end of the buffer.
    ///
    /// Reads past the end return zero rather than panicking, and set this.
    /// GoldSrc's own reader works the same way -- `msg_badread`, visible in the
    /// decompiled `MSG_ReadBitString` below -- and the reason to copy it here
    /// rather than return a `Result` is that `read_n_bit` hands back a
    /// `&BitSlice` borrowed from `self`, which cannot become a `Result` without
    /// rewriting every one of its ~50 call sites.
    ///
    /// Callers that build a `BitReader` check this before returning a parsed
    /// message, so a truncated or spliced demo produces a parse error instead
    /// of an out-of-bounds index. That distinction is load-bearing: the release
    /// profile sets `panic = "abort"`, so a panic in here is not catchable by
    /// the `catch_unwind` in `Analysis::try_from_bytes_with_progress` -- it
    /// takes the whole process down and a folder scan dies with it. See #225.
    pub fn is_bad_read(&self) -> bool {
        self.bad_read
    }

    /// Marks the read as bad from outside, for a caller that has decoded
    /// something structurally impossible rather than run off the end.
    pub fn flag_bad_read(&mut self) {
        self.bad_read = true;
    }

    pub fn read_1_bit(&mut self) -> bool {
        if self.offset >= self.bytes.len() {
            self.bad_read = true;
            return false;
        }
        let res = self.bytes[self.offset];
        self.offset += 1;
        res
    }

    pub fn read_n_bit(&mut self, n: usize) -> &BitSlice {
        let end = match self.offset.checked_add(n) {
            Some(end) if end <= self.bytes.len() => end,
            // Either past the end, or `n` itself is nonsense -- a `bits` field
            // of 0 reaches here as `0usize - 1`, which wraps.
            _ => {
                self.bad_read = true;
                self.offset = self.bytes.len();
                return &self.bytes[self.bytes.len()..];
            }
        };
        let res: &BitSlice = &self.bytes[self.offset..end];
        self.offset = end;
        res
    }

    /*
    char * MSG_ReadBitString(void)
    {
        uint32 uVar1;
        char *pcVar2;

        pcVar2 = MSG_ReadBitString::buf;
        MSG_ReadBitString::buf[0] = '\0';
        if (msg_badread == false) {
        do {
            uVar1 = MSG_ReadBits(8);
            if ((char)uVar1 == '\0') break;
            *pcVar2 = (char)uVar1;
            pcVar2 = pcVar2 + 1;
        } while (msg_badread == false);
        }
        *pcVar2 = '\0';
        return MSG_ReadBitString::buf;
    }
    */
    pub fn read_string(&mut self) -> &BitSlice {
        let start = self.offset;
        let len = self.bytes.len();

        loop {
            if self.offset + 8 > len {
                // Ran out before a null terminator: return what is there and
                // let the caller reject the message.
                self.bad_read = true;
                self.offset = len;
                return &self.bytes[start.min(len)..len];
            }

            let byte = self.bytes[self.offset..self.offset + 8].to_u8();
            // Includes the null terminator.
            self.offset += 8;

            if byte == 0 {
                break;
            }
        }

        &self.bytes[start..self.offset]
    }

    /// Whether `n` more bits can be read without going past the end.
    ///
    /// A loop that runs until it *sees* a terminator needs this: once the
    /// buffer is exhausted, `peek_n_bits` returns an empty slice that reads
    /// back as zero, and a loop whose exit condition is "not the terminator"
    /// would otherwise spin forever, growing a `Vec` until the allocator
    /// aborts. #225.
    pub fn has_bits(&self, n: usize) -> bool {
        matches!(self.offset.checked_add(n), Some(end) if end <= self.bytes.len())
    }

    pub fn peek_n_bits(&self, n: usize) -> &BitSlice {
        match self.offset.checked_add(n) {
            Some(end) if end <= self.bytes.len() => &self.bytes[self.offset..end],
            _ => &self.bytes[self.bytes.len()..],
        }
    }

    pub fn get_offset(&self) -> usize {
        self.offset
    }

    /// Returns the number of bits read into bytes.
    pub fn get_consumed_bytes(&self) -> usize {
        self.get_offset().div_ceil(8)
    }
}

pub struct BitWriter {
    pub data: BitVec,
    pub offset: usize,
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl BitWriter {
    pub fn new() -> Self {
        Self {
            data: BitVec::new(),
            offset: 0,
        }
    }

    fn offset(&mut self, i: usize) {
        self.offset += i;
    }

    pub fn append_bit(&mut self, i: bool) {
        self.data.push(i);
        self.offset(1);
    }

    pub fn append_slice(&mut self, i: &BitSlice) {
        self.data.extend(i);
        self.offset(i.len());
    }

    pub fn append_vec(&mut self, i: &BitVec) {
        self.append_slice(i.as_bitslice())
    }

    pub fn append_u8(&mut self, i: u8) {
        let bits: BitVec = BitVec::from_element(i);
        self.append_vec(&bits);
    }

    /// Append selected bits from a u32.
    /// end = 31 means excluding the sign bit due to LE.
    pub fn append_u32_range(&mut self, i: u32, end: u32) {
        let bits: BitVec = i
            .to_le_bytes()
            .iter()
            .flat_map(|byte| BitVec::from_element(*byte))
            .collect();
        self.append_slice(&bits[..end as usize]);
    }

    pub fn append_i32_range(&mut self, i: i32, end: u32) {
        let bits: BitVec = i
            .to_le_bytes()
            .iter()
            .flat_map(|byte| BitVec::from_element(*byte))
            .collect();
        self.append_slice(&bits[..end as usize]);
    }

    pub fn insert_bit(&mut self, i: bool, pos: usize) {
        self.data.insert(pos, i);
        self.offset(1);
    }

    pub fn insert_slice(&mut self, i: &BitSlice, pos: usize) {
        for (offset, what) in i.iter().enumerate() {
            self.insert_bit(*what, pos + offset);
        }
    }

    pub fn insert_vec(&mut self, i: BitVec, pos: usize) {
        self.insert_slice(i.as_bitslice(), pos);
    }

    pub fn insert_u8(&mut self, i: u8, pos: usize) {
        let bits: BitVec = BitVec::from_element(i);
        self.insert_slice(&bits, pos);
    }

    pub fn insert_u32_range(&mut self, i: u32, end: u32, pos: usize) {
        let bits: BitVec = i
            .to_le_bytes()
            .iter()
            .flat_map(|byte| BitVec::from_element(*byte))
            .collect();

        self.insert_slice(&bits[..end as usize], pos);
    }

    pub fn get_u8_vec(&mut self) -> Vec<u8> {
        // https://github.com/ferrilab/bitvec/issues/27
        let mut what = self.data.to_owned();
        what.force_align();
        what.into_vec()
    }

    pub fn get_offset(&self) -> usize {
        self.offset
    }
}

#[allow(dead_code)]
pub trait BitSliceCast {
    fn to_u8(&self) -> u8;
    fn to_i8(&self) -> i8;
    fn to_u16(&self) -> u16;
    fn to_i16(&self) -> i16;
    fn to_u32(&self) -> u32;
    fn to_i32(&self) -> i32;
    fn get_string(&self) -> String;
}

/// `BitField::load` panics on an empty slice and on one wider than the target,
/// and both are reachable from a corrupted demo: a refused read comes back
/// empty, and a `bits` field read off disk can ask for more bits than the type
/// holds. Neither can happen for a demo that parses, so clamping here changes
/// nothing for valid input and turns an abort into a zero for invalid input.
/// The caller finds out through `BitReader::is_bad_read`. See #225.
macro_rules! load_clamped {
    ($self:expr, $t:ty) => {{
        const WIDTH: usize = <$t>::BITS as usize;
        if $self.is_empty() {
            0 as $t
        } else if $self.len() > WIDTH {
            $self[..WIDTH].load::<$t>()
        } else {
            $self.load::<$t>()
        }
    }};
}

impl BitSliceCast for BitSlice {
    // https://github.com/ferrilab/bitvec/issues/64
    fn to_u8(&self) -> u8 {
        load_clamped!(self, u8)
    }

    fn to_i8(&self) -> i8 {
        load_clamped!(self, i8)
    }

    fn to_u16(&self) -> u16 {
        load_clamped!(self, u16)
    }

    fn to_i16(&self) -> i16 {
        load_clamped!(self, i16)
    }

    fn to_u32(&self) -> u32 {
        load_clamped!(self, u32)
    }

    fn to_i32(&self) -> i32 {
        load_clamped!(self, i32)
    }

    fn get_string(&self) -> String {
        let binding = self
            .chunks(8)
            .map(|chunk| chunk.to_i8() as u8)
            .collect::<Vec<u8>>();

        // Was `from_utf8(..).unwrap()`. A demo carrying a non-UTF-8 byte in a
        // string field is malformed, not a reason to abort the process. #225.
        String::from_utf8_lossy(binding.as_slice()).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads past the end return zero and set the flag, rather than indexing
    /// out of bounds. See `BitReader::is_bad_read` and #225.
    #[test]
    fn reading_past_the_end_sets_bad_read_instead_of_panicking() {
        let mut br = BitReader::new(&[0xff, 0x00]);
        assert_eq!(br.read_n_bit(8).to_u8(), 0xff);
        assert!(!br.is_bad_read());

        assert_eq!(br.read_n_bit(16).to_u8(), 0);
        assert!(br.is_bad_read());
    }

    #[test]
    fn a_single_bit_past_the_end_reads_false() {
        let mut br = BitReader::new(&[0x01]);
        for _ in 0..8 {
            br.read_1_bit();
        }
        assert!(!br.is_bad_read());
        assert!(!br.read_1_bit());
        assert!(br.is_bad_read());
    }

    /// A `bits` field of 0 reaches `read_n_bit` as `0usize - 1`, which wraps.
    #[test]
    fn a_wrapped_bit_count_does_not_index_out_of_bounds() {
        let mut br = BitReader::new(&[0xaa; 4]);
        assert_eq!(br.read_n_bit(usize::MAX).to_u32(), 0);
        assert!(br.is_bad_read());
    }

    #[test]
    fn a_string_with_no_null_terminator_sets_bad_read() {
        let mut br = BitReader::new(b"no terminator here");
        let _ = br.read_string();
        assert!(br.is_bad_read());

        let mut br = BitReader::new(b"terminated\x00");
        assert_eq!(br.read_string().get_string(), "terminated\x00");
        assert!(!br.is_bad_read());
    }

    /// `BitField::load` panics on an empty slice and on one wider than the
    /// target type. Both are reachable from a corrupted demo.
    #[test]
    fn converting_an_empty_or_oversized_slice_does_not_panic() {
        let empty = BitReader::new(&[]);
        assert_eq!(empty.peek_n_bits(8).to_u32(), 0);

        let wide = BitVec::from_slice(&[0xff; 16]);
        assert_eq!(wide.to_u8(), 0xff);
        assert_eq!(wide.to_u32(), u32::MAX);
    }

    #[test]
    fn has_bits_reports_what_is_left() {
        let mut br = BitReader::new(&[0x00, 0x00]);
        assert!(br.has_bits(16));
        assert!(!br.has_bits(17));
        assert!(!br.has_bits(usize::MAX));
        br.read_n_bit(8);
        assert!(br.has_bits(8));
        assert!(!br.has_bits(9));
    }
}
