// Symphonia
// Copyright (c) 2019-2026 The Project Symphonia Developers.
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use symphonia_core::errors::{Error, Result};

use crate::common::FrameHeader;

use super::{GranuleChannel, codebooks, common::*};

use std::cmp::min;
use std::io;
use std::{f32, f64};

use lazy_static::lazy_static;

use log::info;

/// The length of the `POW43` table.
const POW43_LEN: usize = 8207;

lazy_static! {
    /// Lookup table for computing x(i) = s(i)^(4/3) where s(i) is a decoded Huffman sample. The
    /// value of s(i) is bound between 0..8207.
    static ref POW43: Box<[f32; POW43_LEN]> = {
        let pow43: Vec<f32> = (0..POW43_LEN).map(|i| f32::powf(i as f32, 4.0 / 3.0)).collect();
        pow43.into_boxed_slice().try_into().expect("vec initialized to POW43_LEN")
    };
}

/// Zero a sample buffer.
#[inline(always)]
pub(super) fn zero(buf: &mut [f32; 576]) {
    buf.fill(0.0);
}

/// NORI: reads bits most significant first from a byte slice, the next ones in a 64-bit cache filled
/// eight bytes at a time. Past the end it reads zeros, and `count` goes below zero.
struct Bits<'a> {
    buf: &'a [u8],
    /// The next byte to load into `cache`.
    pos: usize,
    /// The unread bits, most significant first. Past `count` it holds zeros or the bits that follow.
    cache: u64,
    /// The unread bits in `cache`, at most 63.
    count: i32,
}

impl<'a> Bits<'a> {
    /// Reads `buf` from bit `at`, at most its length in bits.
    fn new(buf: &'a [u8], at: usize) -> Self {
        let mut bits = Bits { buf, pos: at >> 3, cache: 0, count: 0 };
        bits.refill();
        bits.cache <<= at & 7;
        bits.count -= (at & 7) as i32;
        bits
    }

    /// Loads whole bytes into the cache: at least 56 bits unless the slice ends first.
    #[inline(always)]
    fn refill(&mut self) {
        if let Some(word) = self.buf.get(self.pos..self.pos + 8) {
            let word = u64::from_be_bytes(word.try_into().expect("eight bytes"));
            self.cache |= word >> self.count;
            self.pos += ((63 - self.count) >> 3) as usize;
            self.count |= 56;
        }
        else {
            while self.count <= 55 && self.pos < self.buf.len() {
                self.cache |= u64::from(self.buf[self.pos]) << (56 - self.count);
                self.pos += 1;
                self.count += 8;
            }
        }
    }

    /// The next `n` bits (1 to 32).
    #[inline(always)]
    fn take(&mut self, n: u32) -> u32 {
        let value = (self.cache >> (64 - n)) as u32;
        self.skip(n);
        value
    }

    /// Passes `n` bits (at most 63).
    #[inline(always)]
    fn skip(&mut self, n: u32) {
        self.cache <<= n;
        self.count -= n as i32;
    }

    /// The next bit as a float's sign bit.
    #[inline(always)]
    fn sign(&mut self) -> u32 {
        self.take(1) << 31
    }

    /// Where the next bit is in the slice.
    #[inline(always)]
    fn at(&self) -> usize {
        (8 * self.pos as i64 - i64::from(self.count)) as usize
    }
}

fn end_of_bitstream_error<T>() -> Result<T> {
    Err(Error::IoError(io::Error::other("unexpected end of bitstream")))
}

/// Reads the Huffman coded spectral samples for a given channel in a granule, from bit `start` of
/// `main_data`, into a provided sample buffer. Returns the number of decoded samples (the starting
/// index of the rzero partition).
///
/// Note, each spectral sample is raised to the (4/3)-rd power. This is not actually part of the
/// Huffman decoding process, but, by converting the integer sample to floating point here we don't
/// need to do pointless casting or use an extra buffer.
///
/// NORI: read with `Bits`. Where symphonia_core's reader fails on the first read past the end of
/// `main_data`, this reads zeros and fails once done.
pub(super) fn read_huffman_samples(
    main_data: &[u8],
    start: usize,
    channel: &GranuleChannel,
    part3_bits: u32,
    buf: &mut [f32; 576],
) -> Result<usize> {
    // If there are no Huffman code bits, zero all samples and return immediately.
    if part3_bits == 0 {
        buf.fill(0.0);
        return Ok(0);
    }

    // Dereference the POW43 table once per granule since there is a tiny overhead each time a
    // lazy_static is dereferenced that should be amortized over as many samples as possible.
    let pow43: &[f32; POW43_LEN] = &POW43;

    let mut bs = Bits::new(main_data, start);
    let end = start + part3_bits as usize;
    let mut i = 0;

    // There are two samples per big_value, therefore multiply big_values by 2 to get number of
    // samples in the big_value partition.
    let big_values_len = 2 * channel.big_values as usize;

    // There are up-to 3 regions in the big_value partition. Determine the sample index denoting the
    // end of each region (non-inclusive). Clamp to the end of the big_values partition.
    let regions: [usize; 3] = [
        min(channel.region1_start, big_values_len),
        min(channel.region2_start, big_values_len),
        min(576, big_values_len),
    ];

    // Iterate over each region in big_values.
    for (region_idx, region_end) in regions.iter().enumerate() {
        // Select the Huffman table based on the region's table select value.
        let table_select = channel.table_select[region_idx] as usize;

        // Tables 0..16 are all unique, while tables 16..24 and 24..32 each use one table but
        // differ in the number of linbits to use.
        let codebook = match table_select {
            0..=15 => &codebooks::CODEBOOK_TABLES[table_select],
            16..=23 => &codebooks::CODEBOOK_TABLES[16],
            24..=31 => &codebooks::CODEBOOK_TABLES[17],
            _ => unreachable!(),
        };

        let linbits = codebooks::CODEBOOK_LINBITS[table_select];

        // If the table for a region is empty, fill the region with zeros and move on to the next
        // region.
        if codebook.is_empty() {
            while i < *region_end {
                buf[i] = 0.0;
                i += 1;
                buf[i] = 0.0;
                i += 1;
            }
            continue;
        }

        // Otherwise, read the big_values.
        while i < *region_end && bs.at() < end {
            // A code, two linbits and two sign bits take at most 47 bits.
            if bs.count < 47 {
                bs.refill();
            }

            // Decode the next Huffman code.
            let code = codebook.decode(bs.cache);

            // In the big_values partition, each Huffman code decodes to two sample, x and y. Each
            // sample being 4-bits long.
            let mut x = (code.value() >> 4) as usize;
            let mut y = (code.value() & 0xf) as usize;

            // Without linbits, the sign bits of x and y (each not 0) follow the code.
            if !(linbits > 0 && code.has_15()) {
                let signs = bs.cache << code.len();
                let x_sign = (signs >> 63) as u32 & u32::from(x > 0);
                let y_sign = ((signs << u32::from(x > 0)) >> 63) as u32 & u32::from(y > 0);
                buf[i] = f32::from_bits(pow43[x].to_bits() | x_sign << 31);
                buf[i + 1] = f32::from_bits(pow43[y].to_bits() | y_sign << 31);
                bs.skip(code.len_with_signs());
                i += 2;
                continue;
            }

            bs.skip(code.len());

            // If the first sample, x, is not 0, further process it.
            if x > 0 {
                // If x is saturated (it is at the maximum possible value), and the table specifies
                // linbits, then read linbits more bits and add it to the sample.
                if x == 15 && linbits > 0 {
                    x += bs.take(linbits) as usize;
                }

                // The next bit is the sign bit. If the sign bit is 1, then the sample should be
                // negative. The value of the sample is raised to the (4/3) power.
                buf[i] = f32::from_bits(pow43[x].to_bits() | bs.sign());
            }
            else {
                buf[i] = 0.0;
            }

            i += 1;

            // Likewise, repeat the previous two steps for the second sample, y.
            if y > 0 {
                if y == 15 && linbits > 0 {
                    y += bs.take(linbits) as usize;
                }

                buf[i] = f32::from_bits(pow43[y].to_bits() | bs.sign());
            }
            else {
                buf[i] = 0.0;
            }

            i += 1;
        }
    }

    let count1_codebook = &codebooks::QUADS_CODEBOOK_TABLE[usize::from(channel.count1table_select)];

    // Read the count1 partition.
    while i <= 572 && bs.at() < end {
        // In the count1 partition, each Huffman code decodes to 4 samples: v, w, x, and y.
        // Each sample is 1-bit long (1 or 0).
        //
        // For each 1-bit sample, if it is 0, then the dequantized sample value is 0 as well. If
        // the 1-bit sample is 1, then a sign bit is read. The dequantized sample is then either
        // +/-1.0 depending on the sign bit.
        if bs.count < 47 {
            bs.refill();
        }

        // Decode the next Huffman code.
        let code = count1_codebook.decode(bs.cache);

        // The 4 bits are v, w, x, and y, the first the most significant. Each one set is followed
        // by its sign bit.
        let mut signs = bs.cache << code.len();
        for (k, sample) in buf[i..i + 4].iter_mut().enumerate() {
            let set = (code.value() >> (3 - k)) & 1;
            *sample = f32::from_bits(1f32.to_bits() * set | ((signs >> 63) as u32 & set) << 31);
            signs <<= set;
        }
        bs.skip(code.len_with_signs());

        i += 4;
    }

    // A read past the end of the main data fails.
    if bs.count < 0 {
        return end_of_bitstream_error();
    }

    let bits_read = bs.at() - start;

    // Ignore any extra "stuffing" bits.
    if bits_read < part3_bits as usize {
        if end > 8 * main_data.len() {
            return end_of_bitstream_error();
        }
    }
    // Word on the street is that some encoders are poor at "stuffing" bits, resulting in part3_len
    // being ever so slightly too large. This causes the Huffman decode loop to decode the next few
    // bits as spectral samples. However, these bits are actually random data and are not real
    // samples, therefore, undo them! The caller will be reponsible for re-aligning the bitstream
    // reader. Candy Pop confirms this.
    else if bits_read > part3_bits as usize && i > big_values_len {
        info!("count1 overrun, malformed bitstream");
        i -= 4;
    }
    else if bits_read > part3_bits as usize {
        // It seems that most other decoders don't undo overruns of the big values. We'll just print
        // a message for now.
        info!("big_values overrun, malformed bitstream");
    }

    // The final partition after the count1 partition is the rzero partition. Samples in this
    // partition are all 0.
    buf[i..].fill(0.0);

    Ok(i)
}

/// Requantize long block samples in `buf`.
fn requantize_long(channel: &GranuleChannel, bands: &[usize], buf: &mut [f32; 576]) {
    // For long blocks dequantization and scaling is governed by the following equation:
    //
    //                     xr(i) = s(i)^(4/3) * 2^(0.25*A) * 2^(-B)
    // where:
    //       s(i) is the decoded Huffman sample
    //      xr(i) is the dequantized sample
    // and:
    //      A = global_gain[gr] - 210
    //      B = scalefac_multiplier * (scalefacs[gr][ch][sfb] + (preflag[gr] * pretab[sfb]))
    //
    // Note: The samples in buf are the result of s(i)^(4/3) for each sample i.
    debug_assert!(bands.len() <= 23);

    // The preemphasis table is from table B.6 in ISO/IEC 11172-3.
    const PRE_EMPHASIS: [u8; 22] =
        [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 3, 2, 0];

    // Calculate A, it is constant for the entire requantization.
    let a = i32::from(channel.global_gain) - 210;

    let scalefac_shift = if channel.scalefac_scale { 2 } else { 1 };

    // Requantize each scale-factor band in buf.
    for (i, (start, end)) in bands.iter().zip(&bands[1..]).enumerate() {
        // Do not requantize bands starting after the rzero sample since all samples from there on
        // are 0.
        if *start >= channel.rzero {
            break;
        }

        // Lookup the pre-emphasis amount if required.
        let pre_emphasis = if channel.preflag { PRE_EMPHASIS[i] } else { 0 };

        // Calculate B.
        let b = i32::from((channel.scalefacs[i] + pre_emphasis) << scalefac_shift);

        // Calculate 2^(0.25*A) * 2^(-B). This can be rewritten as 2^{ 0.25 * (A - 4 * B) }.
        // Since scalefac_shift was multiplies by 4 above, the final equation becomes
        // 2^{ 0.25 * (A - B) }.
        let pow2ab = f64::powf(2.0, 0.25 * f64::from(a - b)) as f32;

        // Calculate the ending sample index for the scale-factor band, clamping it to the length of
        // the sample buffer.
        let band_end = min(*end, channel.rzero);

        // The sample buffer contains s(i)^(4/3), now multiply in 2^(0.25*A) * 2^(-B) to get xr(i).
        for sample in &mut buf[*start..band_end] {
            *sample *= pow2ab;
        }
    }
}

/// Requantize short block samples in `buf` starting at scale-factor band `sfb_init`.
fn requantize_short(
    channel: &GranuleChannel,
    bands: &[usize],
    switch: usize,
    buf: &mut [f32; 576],
) {
    // For short blocks dequantization and scaling is governed by the following equation:
    //
    //                     xr(i) = s(i)^(4/3) * 2^(0.25*A) * 2^(-B)
    // where:
    //       s(i) is the decoded Huffman sample
    //      xr(i) is the dequantized sample
    // and:
    //      A = global_gain[gr] - 210 - (8 * subblock_gain[gr][win])
    //      B = scalefac_multiplier * scalefacs[gr][ch][sfb][win]
    //
    // Note: The samples in buf are the result of s(i)^(4/3) for each sample i.
    debug_assert!(bands.len() <= 40);

    // Calculate the window-independant part of A: global_gain[gr] - 210.
    let gain = i32::from(channel.global_gain) - 210;

    // Calculate A for each window.
    let a = [
        gain - 8 * i32::from(channel.subblock_gain[0]),
        gain - 8 * i32::from(channel.subblock_gain[1]),
        gain - 8 * i32::from(channel.subblock_gain[2]),
    ];

    // Likweise, the scalefac_multiplier is constant for the granule. The actual scale is multiplied
    // by 4 to combine the two pow2 operations into one by adding the exponents. The sum of the
    // exponent is multiplied by 0.25 so B must be multiplied by 4 to counter the quartering. A
    // bitshift operation is used for the actual multiplication, so scalefac_multiplier is named
    // scalefac_shift in this case.
    let scalefac_shift = if channel.scalefac_scale { 2 } else { 1 };

    for (i, (start, end)) in bands.iter().zip(&bands[1..]).enumerate() {
        // Do not requantize bands starting after the rzero sample since all samples from there on
        // are 0.
        if *start >= channel.rzero {
            break;
        }

        // Calculate B.
        let b = i32::from(channel.scalefacs[switch + i] << scalefac_shift);

        // Calculate 2^(0.25*A) * 2^(-B). This can be rewritten as 2^{ 0.25 * (A - 4 * B) }.
        // Since scalefac_shift multiplies by 4 above, the final equation becomes
        // 2^{ 0.25 * (A - B) }.
        let pow2ab = f64::powf(2.0, 0.25 * f64::from(a[i % 3] - b)) as f32;

        // Clamp the ending sample index to the rzero sample index. Since samples starting from
        // rzero are 0, there is no point in requantizing them.
        let win_end = min(*end, channel.rzero);

        // The sample buffer contains s(i)^(4/3), now multiply in 2^(0.25*A) * 2^(-B) to get
        // xr(i).
        for sample in &mut buf[*start..win_end] {
            *sample *= pow2ab;
        }
    }
}

/// Requantize samples in `buf` regardless of block type.
pub(super) fn requantize(header: &FrameHeader, channel: &GranuleChannel, buf: &mut [f32; 576]) {
    match channel.block_type {
        BlockType::Short { is_mixed: false } => {
            requantize_short(channel, &SFB_SHORT_BANDS[header.sample_rate_idx], 0, buf);
        }
        BlockType::Short { is_mixed: true } => {
            // A mixed block is a combination of a long block and short blocks. The first few scale
            // factor bands, and thus samples, belong to a single long block, while the remaining
            // bands and samples belong to short blocks. Therefore, requantization for mixed blocks
            // can be decomposed into short and long block requantizations.
            //
            // As per ISO/IEC 11172-3, the short scale factor band at which the long block ends and
            // the short blocks begin is denoted by switch_point_s.
            let bands = SFB_MIXED_BANDS[header.sample_rate_idx];
            let switch = SFB_MIXED_SWITCH_POINT[header.sample_rate_idx];

            requantize_long(channel, &bands[..switch], buf);
            requantize_short(channel, &bands[switch..], switch, buf);
        }
        _ => {
            requantize_long(channel, &SFB_LONG_BANDS[header.sample_rate_idx], buf);
        }
    }
}
