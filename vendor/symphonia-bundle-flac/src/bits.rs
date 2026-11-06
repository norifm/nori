// NORI: a bit reader for subframes that fills its cache eight bytes at a time, and reads a Rice
// partition in one loop. It reads what symphonia_core's BitReaderLtr reads, and fails where it fails.

use std::io;

use crate::decoder::rice_signed_to_i32;

fn end_of_bitstream_error<T>() -> io::Result<T> {
    Err(io::Error::other("unexpected end of bitstream"))
}

/// Reads bits most significant first from a byte slice.
pub(crate) struct Bits<'a> {
    buf: &'a [u8],
    /// The next byte to load into `cache`.
    pos: usize,
    /// The unread bits, most significant first. Past `count` it holds zeros or the bits that follow.
    cache: u64,
    /// The unread bits in `cache`, at most 63.
    count: u32,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Bits { buf, pos: 0, cache: 0, count: 0 }
    }

    /// Loads whole bytes into the cache: at least 56 bits unless the slice ends first.
    #[inline(always)]
    fn refill(&mut self) {
        if let Some(word) = self.buf.get(self.pos..self.pos + 8) {
            let word = u64::from_be_bytes(word.try_into().expect("eight bytes"));
            self.cache |= word >> self.count;
            let bytes = (63 - self.count) >> 3;
            self.pos += bytes as usize;
            self.count += bytes << 3;
        }
        else {
            self.refill_end();
        }
    }

    #[cold]
    fn refill_end(&mut self) {
        while self.count <= 55 && self.pos < self.buf.len() {
            self.cache |= u64::from(self.buf[self.pos]) << (56 - self.count);
            self.pos += 1;
            self.count += 8;
        }
    }

    #[inline(always)]
    fn consume(&mut self, n: u32) {
        self.cache <<= n;
        self.count -= n;
    }

    #[inline(always)]
    pub(crate) fn read_bool(&mut self) -> io::Result<bool> {
        Ok(self.read_bits_leq32(1)? == 1)
    }

    #[inline(always)]
    pub(crate) fn read_bits_leq32(&mut self, n: u32) -> io::Result<u32> {
        if self.count < n {
            self.refill();
            if self.count < n {
                return end_of_bitstream_error();
            }
        }
        let value = ((self.cache >> 32) >> (32 - n)) as u32;
        self.consume(n);
        Ok(value)
    }

    /// Reads zeros up to a one, and returns how many.
    pub(crate) fn read_unary_zeros(&mut self) -> io::Result<u32> {
        let mut zeros = 0;
        loop {
            let run = self.cache.leading_zeros();
            if run < self.count {
                self.consume(run + 1);
                return Ok(zeros + run);
            }
            zeros += self.count;
            self.consume(self.count);
            self.refill();
            if self.count == 0 {
                return end_of_bitstream_error();
            }
        }
    }

    /// Reads Rice codes with parameter `k` (at most 30) into `out` as signed residuals.
    pub(crate) fn read_rice(&mut self, k: u32, out: &mut [i32]) -> io::Result<()> {
        // The cache lives in locals here, so the loop keeps it in registers.
        let Bits { buf, mut pos, mut cache, mut count } = *self;
        for sample in out.iter_mut() {
            if count < 32 {
                let Some(word) = buf.get(pos..pos + 8)
                else {
                    *self = Bits { buf, pos, cache, count };
                    *sample = self.read_rice_one(k)?;
                    (pos, cache, count) = (self.pos, self.cache, self.count);
                    continue;
                };
                cache |= u64::from_be_bytes(word.try_into().expect("eight bytes")) >> count;
                pos += ((63 - count) >> 3) as usize;
                count |= 56;
            }
            let zeros = cache.leading_zeros();
            let len = zeros + 1 + k;
            if len > count {
                *self = Bits { buf, pos, cache, count };
                *sample = self.read_rice_one(k)?;
                (pos, cache, count) = (self.pos, self.cache, self.count);
                continue;
            }
            // The stop bit leads, then the k low bits.
            let stop = cache << zeros;
            let low = (stop << 1 >> 1) >> (63 - k);
            cache = stop << (1 + k);
            count -= len;
            *sample = rice_signed_to_i32((zeros << k) | low as u32);
        }
        *self = Bits { buf, pos, cache, count };
        Ok(())
    }

    #[cold]
    fn read_rice_one(&mut self, k: u32) -> io::Result<i32> {
        let q = self.read_unary_zeros()?;
        Ok(rice_signed_to_i32((q << k) | self.read_bits_leq32(k)?))
    }
}
