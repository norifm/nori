// NORI: a bit reader for a packet that fills its cache eight bytes at a time, and reads a channel's
// residuals in one loop, each of ALAC's adaptive Golomb codes from the cache in one step. It reads what
// symphonia_core's BitReaderLtr reads, and fails where it fails.

use std::io;

fn end_of_bitstream_error<T>() -> io::Result<T> {
    Err(io::Error::other("unexpected end of bitstream"))
}

/// Reads bits most significant first from a byte slice.
#[derive(Clone, Copy)]
pub(crate) struct Bits<'a> {
    buf: &'a [u8],
    /// The next byte to load into `cache`.
    pos: usize,
    /// The unread bits, most significant first. Past `count` it holds zeros or the bits that follow.
    cache: u64,
    /// The unread bits in `cache`, at most 63.
    count: u32,
}

/// How a channel's residuals are coded: the history's start (`mb`) and rate of change (`pb_factor`),
/// the most `k` may be (`kb`), and the bits of an escaped residual (`bps`).
pub(crate) struct Golomb {
    pub(crate) mb: u32,
    pub(crate) kb: u32,
    pub(crate) pb_factor: u32,
    pub(crate) bps: u32,
}

impl<'a> Bits<'a> {
    pub(crate) fn new(buf: &'a [u8]) -> Self {
        Bits { buf, pos: 0, cache: 0, count: 0 }
    }

    /// Loads whole bytes into the cache while eight remain, leaving at least 56 bits; false if fewer
    /// remain.
    #[inline(always)]
    fn refill_word(&mut self) -> bool {
        let Some(word) = self.buf.get(self.pos..self.pos + 8)
        else {
            return false;
        };
        self.cache |= u64::from_be_bytes(word.try_into().expect("eight bytes")) >> self.count;
        self.pos += ((63 - self.count) >> 3) as usize;
        self.count |= 56;
        true
    }

    /// Loads whole bytes into the cache: at least 56 bits unless the slice ends first.
    #[inline(always)]
    fn refill(&mut self) {
        if !self.refill_word() {
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

    /// Discards the bits up to the next byte.
    pub(crate) fn realign(&mut self) {
        self.consume(self.count & 7);
    }

    pub(crate) fn ignore_bits(&mut self, mut n: u32) -> io::Result<()> {
        while n > self.count {
            n -= self.count;
            self.consume(self.count);
            self.refill();
            if self.count == 0 {
                return end_of_bitstream_error();
            }
        }
        self.consume(n);
        Ok(())
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

    #[inline(always)]
    pub(crate) fn read_bits_leq32_signed(&mut self, n: u32) -> io::Result<i32> {
        let value = self.read_bits_leq32(n)?;
        Ok((value.wrapping_shl(32 - n) as i32).wrapping_shr(32 - n))
    }

    /// Reads a channel's residuals into `out`, which holds zeros: a Golomb code each, but where the
    /// history falls under 128 a code says how many zeros follow.
    pub(crate) fn read_residuals(&mut self, out: &mut [i32], golomb: &Golomb) -> io::Result<()> {
        let Golomb { mut mb, kb, pb_factor, bps } = *golomb;
        // A local copy, so the loop keeps the cache in registers; `self` reads near the end.
        let mut bits = *self;
        let mut sign_toggle = 0;
        let mut i = 0;
        while i < out.len() {
            let k = lg3a(mb).min(kb);
            let code = match bits.read_golomb_cached(k, bps) {
                Some(code) => code,
                None => {
                    *self = bits;
                    let code = self.read_golomb(k, bps)?;
                    bits = *self;
                    code
                }
            };
            let val = code.wrapping_add(sign_toggle);
            out[i] = rice_code_to_signed(val);
            if val > 0xffff {
                mb = 0xffff;
            }
            else {
                // If `val` and `pb_factor` are forced to be very large by the bitstream, their
                // product can't overflow a u32, but it can overflow `mb` when added over multiple
                // iterations.
                mb = mb.wrapping_add(pb_factor * val).wrapping_sub(pb_factor.wrapping_mul(mb) >> 9);
            }
            sign_toggle = 0;
            i += 1;
            if mb < 128 && i < out.len() {
                // This subtraction cannot overflow because mb is a u32 and < 128.
                let k = (mb.leading_zeros() - 24 + ((mb + 16) >> 6)).min(kb);
                let zeros = match bits.read_golomb_cached(k, 16) {
                    Some(zeros) => zeros,
                    None => {
                        *self = bits;
                        let zeros = self.read_golomb(k, 16)?;
                        bits = *self;
                        zeros
                    }
                };
                if zeros < 0xffff {
                    sign_toggle = 1;
                }
                mb = 0;
                // The run's samples are already zeros.
                i += zeros as usize;
            }
        }
        *self = bits;
        Ok(())
    }

    /// `read_golomb` from the cache in one step; None where the cache may not hold the code (near
    /// the packet's end) and for `k` 0.
    #[inline(always)]
    fn read_golomb_cached(&mut self, k: u32, bps: u32) -> Option<u32> {
        // At least 56 bits then, as a code takes at most 9 + 32.
        if !self.refill_word() || k == 0 {
            return None;
        }
        let ones = self.cache.leading_ones();
        if ones > 8 {
            let value = ((self.cache << 9) >> 32 >> (32 - bps)) as u32;
            self.consume(9 + bps);
            return Some(value);
        }
        // The k bits after the zero; a value under 2 takes k - 1 of them.
        let low = ((self.cache << (ones + 1)) >> (64 - k)) as u32;
        self.consume(ones + k + u32::from(low > 1));
        Some((ones << k) - ones + low.max(1) - 1)
    }

    /// Reads one of ALAC's Golomb codes with parameter `k` (at most 23) as symphonia does, a field at
    /// a time: up to eight ones and a zero, then `k` bits, of which the last is left unread when the
    /// others are zeros; or nine ones and a value of `bps` bits (at most 32).
    #[cold]
    fn read_golomb(&mut self, k: u32, bps: u32) -> io::Result<u32> {
        let prefix = self.read_unary_ones_capped(9)?;
        Ok(if prefix > 8 {
            self.read_bits_leq32(bps)?
        }
        else if k > 1 {
            let value = (prefix << k) - prefix;
            let suffix = self.read_bits_leq32(k - 1)?;
            if suffix > 0 { value + (suffix << 1) + self.read_bits_leq32(1)? - 1 } else { value }
        }
        else if k == 1 {
            prefix
        }
        else {
            0
        })
    }

    /// Reads ones up to a zero, which is read too, or up to `limit` ones; returns how many ones.
    fn read_unary_ones_capped(&mut self, limit: u32) -> io::Result<u32> {
        let mut num = 0;
        loop {
            let ones = self.cache.leading_ones().min(self.count);
            if num + ones >= limit {
                self.consume(limit - num);
                return Ok(limit);
            }
            if ones < self.count {
                self.consume(ones + 1);
                return Ok(num + ones);
            }
            num += ones;
            self.consume(ones);
            self.refill();
            if self.count == 0 {
                return end_of_bitstream_error();
            }
        }
    }
}

#[inline(always)]
fn lg3a(val: u32) -> u32 {
    31 - ((val >> 9) + 3).leading_zeros()
}

/// Converts the unsigned rice code into a signed i32.
#[inline(always)]
fn rice_code_to_signed(val: u32) -> i32 {
    // The last bit of the decoded rice value is the sign-bit. See FLAC decoder for a derivation
    // of this function.
    (val >> 1) as i32 ^ -((val & 0x1) as i32)
}
