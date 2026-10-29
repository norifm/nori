//! How Nori plays music, platform-free: decoding, the sound chain, AutoMix, transitions, the queue and
//! the player pipeline. Platforms hand in packets and settings and get PCM and decisions back.
//! Without `std`, only the sound chain's DSP and the records it is set with.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

#[cfg(not(feature = "std"))]
mod math;
mod prelude;

#[cfg(feature = "std")]
pub mod automix;
#[cfg(feature = "std")]
pub mod burst;
#[cfg(feature = "std")]
mod chain;
pub mod compressor;
pub mod dac;
pub mod device;
#[cfg(feature = "std")]
pub mod decode;
pub mod dither;
pub mod dsp;
#[cfg(feature = "std")]
pub mod engine;
pub mod eqfit;
pub mod gain;
pub mod graphic;
pub mod outputs;
#[cfg(feature = "std")]
pub mod heard;
#[cfg(feature = "std")]
pub mod pcm;
#[cfg(feature = "std")]
pub mod pipeline;
#[cfg(feature = "std")]
pub mod playlist;
pub mod policy;
#[cfg(feature = "std")]
pub mod queue;
#[cfg(feature = "std")]
pub mod silence;
#[cfg(feature = "std")]
pub mod sink;
#[cfg(any(test, feature = "synth"))]
pub mod sim;
#[cfg(feature = "std")]
pub mod sonic;
pub mod spatial;
#[cfg(feature = "std")]
pub mod speed;
#[cfg(feature = "std")]
pub mod stage;
#[cfg(feature = "std")]
pub mod transitions;
pub mod transport;
pub mod types;

/// Allocation checks for the audio path.
#[cfg(test)]
mod no_alloc;
