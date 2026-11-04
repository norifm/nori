// NORI: a bit reader for the residues that fills its cache eight bytes at a time, and looks a short
// code up in one step (`FastCodes`). It reads what symphonia_core's BitReaderRtl reads, and fails
// where it fails.

use symphonia_core::io::{BitReaderRtl, ReadBitsRtl};

/// Reads bits least significant first from a byte slice.
#[derive(Clone, Copy)]
pub(crate) struct Bits<'a> {
    buf: &'a [u8],
    /// The next byte to load into `cache`.
    pos: usize,
    /// The unread bits, the next lowest. Past `count` it holds zeros or the bits that follow.
    cache: u64,
    /// The unread bits in `cache`, at most 63.
    count: u32,
}

impl<'a> Bits<'a> {
    /// A reader from bit `at` of `buf`, which is at most its length.
    pub(crate) fn at(buf: &'a [u8], at: u64) -> Self {
        let mut bits = Bits { buf, pos: (at / 8) as usize, cache: 0, count: 0 };
        bits.refill();
        bits.consume((at % 8) as u32);
        bits
    }

    /// The bits read from the start of the slice.
    pub(crate) fn position(&self) -> u64 {
        self.pos as u64 * 8 - u64::from(self.count)
    }

    /// Loads whole bytes into the cache: at least 56 bits unless the slice ends first.
    #[inline(always)]
    fn refill(&mut self) {
        if let Some(word) = self.buf.get(self.pos..self.pos + 8) {
            self.cache |= u64::from_le_bytes(word.try_into().expect("eight bytes")) << self.count;
            self.pos += ((63 - self.count) >> 3) as usize;
            self.count |= 56;
        }
        else {
            self.refill_end();
        }
    }

    #[cold]
    fn refill_end(&mut self) {
        while self.count <= 55 && self.pos < self.buf.len() {
            self.cache |= u64::from(self.buf[self.pos]) << self.count;
            self.pos += 1;
            self.count += 8;
        }
    }

    #[inline(always)]
    fn consume(&mut self, n: u32) {
        self.cache >>= n;
        self.count -= n;
    }

    /// The value of the next code in `codes`, if it is one of its short codes and all its bits are
    /// left; None otherwise, with nothing read.
    #[inline(always)]
    pub(crate) fn read_fast(&mut self, codes: FastCodesRef<'_>) -> Option<u32> {
        if self.count < FAST_BITS {
            self.refill();
        }
        let entry = codes.table[(self.cache & codes.mask) as usize];
        let len = entry & 0xff;
        if len == 0 || len > self.count {
            return None;
        }
        self.consume(len);
        Some(entry >> 8)
    }

    /// symphonia's reader at this position.
    pub(crate) fn reader(&self) -> BitReaderRtl<'a> {
        let at = self.position();
        let mut reader = BitReaderRtl::new(&self.buf[(at / 8) as usize..]);
        // Fewer than eight bits, of a byte that is there.
        reader.ignore_bits((at % 8) as u32).expect("the rest of a byte that is there");
        reader
    }

    /// Steps over `n` bits read by `reader`.
    pub(crate) fn skip(&mut self, mut n: u32) {
        while n > self.count {
            n -= self.count;
            self.consume(self.count);
            self.refill();
        }
        self.consume(n);
    }
}

/// The most bits a short code has.
const FAST_BITS: u32 = 10;

/// A codebook's codes of up to `FAST_BITS` bits, by the next bits read: a value above its code's
/// length, or 0 for a longer code.
pub(crate) struct FastCodes {
    table: Box<[u32]>,
    mask: u64,
}

/// `FastCodes` borrowed, which a loop keeps in registers.
#[derive(Clone, Copy)]
pub(crate) struct FastCodesRef<'a> {
    table: &'a [u32],
    mask: u64,
}

impl FastCodes {
    #[inline(always)]
    pub(crate) fn borrow(&self) -> FastCodesRef<'_> {
        FastCodesRef { table: &self.table, mask: self.mask }
    }

    /// From a codebook's codewords (first bit most significant), their lengths and values.
    pub(crate) fn new(codes: &[u32], lens: &[u8], values: &[u32]) -> Self {
        let bits = lens.iter().map(|&len| u32::from(len)).max().unwrap_or(0).min(FAST_BITS);
        let mut table = vec![0; 1 << bits].into_boxed_slice();
        for ((&code, &len), &value) in codes.iter().zip(lens).zip(values) {
            let len = u32::from(len);
            if len == 0 || len > bits {
                continue;
            }
            // Read least significant first, the code's first bit is the lowest.
            let first = code.reverse_bits() >> (32 - len);
            for rest in 0..1u32 << (bits - len) {
                table[(first | (rest << len)) as usize] = (value << 8) | len;
            }
        }
        FastCodes { table, mask: (1 << bits) - 1 }
    }
}
