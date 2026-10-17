//! The browser's own decoders (WebCodecs, through `web/src/decoder.ts`) as the engine's decoders that
//! answer later: a song's packets are fed ahead, and each one's samples taken back once decoded. A codec
//! the browser lacks and this module does not decode (Vorbis, ALAC, Opus in `player.wasm`) is decoded by
//! the Rust decoders of `player-codecs.wasm`, loaded beside it once a song needs them (`Codecs`).

use nori_player::decode::{Codec, Fault, LaterDecoder, Setup};
use wasm_bindgen::prelude::*;

#[wasm_bindgen(raw_module = "../decoder.js")]
extern "C" {
    type Decoding;

    #[wasm_bindgen(static_method_of = Decoding)]
    fn make(codec: &str, rate: u32, channels: u32, config: Option<Box<[u8]>>) -> Option<Decoding>;
    /// The same over `player-codecs.wasm`'s `Codecs`, loaded on first use.
    #[wasm_bindgen(static_method_of = Decoding)]
    fn rust(codec: &str, rate: u32, channels: u32, config: Option<Box<[u8]>>) -> Decoding;
    #[wasm_bindgen(method)]
    fn feed(this: &Decoding, unit: &[u8], at_us: f64);
    #[wasm_bindgen(method)]
    fn ready(this: &Decoding) -> bool;
    #[wasm_bindgen(method)]
    fn samples(this: &Decoding) -> i32;
    #[wasm_bindgen(method)]
    fn channels(this: &Decoding) -> u32;
    #[wasm_bindgen(method)]
    fn rate(this: &Decoding) -> u32;
    #[wasm_bindgen(method)]
    fn take(this: &Decoding, out: &mut [f32]);
    #[wasm_bindgen(method)]
    fn finish(this: &Decoding);
    #[wasm_bindgen(method)]
    fn reset(this: &Decoding);
    #[wasm_bindgen(method)]
    fn free(this: &Decoding);
}

struct Web(Decoding);

// SAFETY: wasm32-unknown-unknown without the atomics feature has a single thread; nothing is ever sent.
unsafe impl Send for Web {}

impl LaterDecoder for Web {
    fn feed(&mut self, unit: &[u8], at_us: i64) {
        self.0.feed(unit, at_us as f64);
    }

    fn ready(&self) -> bool {
        self.0.ready()
    }

    fn take(&mut self, out: &mut Vec<f32>) -> Option<Result<(usize, u32), Fault>> {
        if !self.0.ready() {
            return None;
        }
        let shape = (self.0.channels() as usize, self.0.rate());
        let Ok(n) = usize::try_from(self.0.samples()) else { return Some(Err(Fault::Broken)) };
        let at = out.len();
        out.resize(at + n, 0.0);
        self.0.take(&mut out[at..]);
        Some(Ok(shape))
    }

    fn finish(&mut self) {
        self.0.finish();
    }

    fn reset(&mut self) {
        self.0.reset();
    }
}

impl Drop for Web {
    fn drop(&mut self) {
        self.0.free();
    }
}

fn name(codec: Codec) -> &'static str {
    match codec {
        Codec::Mp3 => "mp3",
        Codec::Flac => "flac",
        Codec::Aac => "aac",
        Codec::Vorbis => "vorbis",
        Codec::Opus => "opus",
        Codec::Alac => "alac",
    }
}

/// The codecs this module decodes only through `player-codecs.wasm` (that one has them itself).
fn beside(codec: Codec) -> bool {
    !cfg!(feature = "rust-codecs") && matches!(codec, Codec::Vorbis | Codec::Alac | Codec::Opus)
}

/// The browser's decoder for `setup`'s stream, else `player-codecs.wasm`'s where only that has one
/// ([`nori_player::decode::PlatformLater`]); None where this module decodes the stream itself.
pub fn decoder(setup: &Setup) -> Option<Box<dyn LaterDecoder>> {
    let (codec, rate, channels, config) = (name(setup.codec), setup.rate, setup.channels as u32, setup.config.map(Box::from));
    let d = match Decoding::make(codec, rate, channels, config.clone()) {
        Some(d) => d,
        None if beside(setup.codec) => Decoding::rust(codec, rate, channels, config),
        None => return None,
    };
    Some(Box::new(Web(d)))
}

/// `player-codecs.wasm`'s Rust decoders for another module: a packet in, its samples out, interleaved.
#[cfg(feature = "rust-codecs")]
#[wasm_bindgen]
pub struct Codecs {
    dec: nori_player::decode::Decoder,
    /// The last packet's shape.
    channels: u32,
    rate: u32,
}

#[cfg(feature = "rust-codecs")]
#[wasm_bindgen]
impl Codecs {
    /// A decoder for `codec` (as `decoder` names them), or None.
    pub fn make(codec: &str, rate: u32, channels: u32, config: Option<Box<[u8]>>) -> Option<Codecs> {
        let codec = [Codec::Vorbis, Codec::Alac, Codec::Opus].into_iter().find(|&c| name(c) == codec)?;
        let dec = nori_player::decode::Decoder::new(codec, rate, channels as usize, config.as_deref(), false).ok()?;
        Some(Codecs { channels: dec.channels() as u32, rate: dec.rate(), dec })
    }

    /// One packet's samples (none for a bad packet); None once the decoder broke.
    pub fn decode(&mut self, unit: &[u8]) -> Option<Box<[f32]>> {
        match self.dec.decode_lent(unit) {
            Ok(lent) => {
                (self.channels, self.rate) = (lent.channels as u32, lent.rate);
                Some(lent.samples.into())
            }
            Err(Fault::Broken) => None,
            Err(_) => Some(Box::new([])),
        }
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    /// A seek: the decoder starts afresh, dropping the codec's own priming again as a browser's does
    /// (Opus's pre-skip; the reader drops the rest of its pre-roll).
    pub fn reset(&mut self) {
        self.dec.reset(true);
    }
}
