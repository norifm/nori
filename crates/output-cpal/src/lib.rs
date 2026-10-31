//! `nori-engine`'s desktop output over cpal (PipeWire/ALSA, CoreAudio, WASAPI).
//!
//! The device is asked for the stream's rate and channels first so nothing is resampled; otherwise it
//! plays at its own and the engine converts. Paused, the stream is stopped so the device can sleep.
//! The engine is told the current device on open and when the system moves the default stream
//! (not reported on ALSA).

#[cfg(target_os = "macos")]
mod macos;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, Device, DeviceType, ErrorKind, InterfaceType, SampleFormat, SizedSample, Stream, StreamConfig};
use nori_engine::{AudioOutput, DeviceWatch, Feed, Heard, OutputFormat, OutputKind};
use parking_lot::Mutex;

#[derive(Default)]
pub struct CpalOutput {
    /// Device name; None is the system default.
    wanted: Option<String>,
    device: Option<Device>,
    config: Option<(StreamConfig, SampleFormat)>,
    /// Period range the device supports, frames.
    periods: Option<(u32, u32)>,
    /// The engine's side of the stream. Only the playing stream's callback locks it.
    feed: Option<Arc<Mutex<Feed>>>,
    stream: Option<Stream>,
    playing: bool,
    /// When what the device took has been heard, on [`clock_us`].
    heard: Arc<Heard>,
    /// Why the stream stopped for good (its device went away), for [`AudioOutput::failed`].
    failed: Arc<Mutex<Option<String>>>,
    watch: Option<Arc<DeviceWatch>>,
    volume: Volume,
}

/// This output's clock for [`Heard`], µs from an arbitrary base.
fn clock_us() -> u64 {
    static BASE: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(Instant::now).elapsed().as_micros() as u64
}

/// Listener volume, 0 to 1, applied after the engine's chain so ReplayGain, limiter and fades see full
/// scale. Settable from any thread.
#[derive(Clone)]
pub struct Volume(Arc<AtomicU32>);

impl Volume {
    pub fn set(&self, v: f32) {
        self.0.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }
}

impl Default for Volume {
    fn default() -> Self {
        Volume(Arc::new(AtomicU32::new(1.0f32.to_bits())))
    }
}

fn describe(device: &Device) -> Option<nori_engine::Device> {
    let d = device.description().ok()?;
    let kind = match (d.interface_type(), d.device_type()) {
        (InterfaceType::Usb, _) => OutputKind::Usb,
        (InterfaceType::Bluetooth, _) => OutputKind::Bluetooth,
        (InterfaceType::Hdmi | InterfaceType::DisplayPort | InterfaceType::Line | InterfaceType::Spdif, _) => OutputKind::Line,
        (_, DeviceType::Headphones | DeviceType::Headset) => OutputKind::Wired,
        (_, DeviceType::Speaker) | (InterfaceType::BuiltIn, _) => OutputKind::Speaker,
        (_, DeviceType::Dock) => OutputKind::Line,
        _ => OutputKind::Other,
    };
    #[cfg(target_os = "macos")]
    let kind = macos::kind_of(d.name()).unwrap_or(kind);
    let device = nori_engine::Device { kind, name: d.name().to_string() };
    #[cfg(target_os = "linux")]
    if device.name == SYSTEM_DEFAULT {
        return Some(system_output().unwrap_or(device));
    }
    Some(device)
}

/// What cpal calls ALSA's default, whichever sound card the sound server plays on.
#[cfg(target_os = "linux")]
const SYSTEM_DEFAULT: &str = "Default Audio Device";

/// The sound server's default sink: what the music really plays on. Asked as an output opens.
#[cfg(target_os = "linux")]
fn system_output() -> Option<nori_engine::Device> {
    let pactl = |args: &[&str]| {
        let out = std::process::Command::new("pactl").args(args).output().ok().filter(|o| o.status.success())?;
        String::from_utf8(out.stdout).ok()
    };
    sink(&pactl(&["list", "sinks"])?, pactl(&["get-default-sink"])?.trim())
}

/// Sink `name` of what `pactl list sinks` said, as an output: its description without the profile's
/// words, its kind from the name's bus.
#[cfg(any(target_os = "linux", test))]
fn sink(sinks: &str, name: &str) -> Option<nori_engine::Device> {
    let lines = sinks.lines().map(str::trim);
    let description = lines.skip_while(|l| l.strip_prefix("Name: ") != Some(name)).find_map(|l| l.strip_prefix("Description: "))?;
    let bare = [" Analog ", " Digital ", " Pro"].iter().fold(description, |d, profile| d.split(profile).next().unwrap_or(d));
    let kind = match () {
        _ if name.starts_with("bluez_output.") => OutputKind::Bluetooth,
        _ if name.contains(".usb-") => OutputKind::Usb,
        _ if name.contains(".hdmi") => OutputKind::Line,
        _ => OutputKind::Other,
    };
    Some(nori_engine::Device { kind, name: bare.trim().to_string() })
}

impl CpalOutput {
    /// The system's default output device.
    pub fn new() -> CpalOutput {
        CpalOutput::default()
    }

    /// The output device called `name`, as [`CpalOutput::devices`] lists it.
    pub fn with_device(name: &str) -> CpalOutput {
        CpalOutput { wanted: Some(name.to_string()), ..CpalOutput::default() }
    }

    pub fn volume(&self) -> Volume {
        self.volume.clone()
    }

    /// At the level `volume` sets (a card made again keeps the session's volume).
    pub fn with_volume(self, volume: Volume) -> CpalOutput {
        CpalOutput { volume, ..self }
    }

    pub fn devices() -> Vec<String> {
        let host = cpal::default_host();
        host.output_devices().map(|ds| ds.filter_map(|d| d.description().ok().map(|n| n.to_string())).collect()).unwrap_or_default()
    }

    fn pick(&self) -> Result<Device, String> {
        let host = cpal::default_host();
        match &self.wanted {
            None => host.default_output_device().ok_or_else(|| "no output device".to_string()),
            Some(name) => host
                .output_devices()
                .map_err(|e| e.to_string())?
                .find(|d| d.description().is_ok_and(|n| n.to_string() == *name))
                .ok_or_else(|| format!("no output device called {name}")),
        }
    }
}

/// Preference among supported sample formats; None is unsupported.
fn rank(f: SampleFormat) -> Option<u8> {
    match f {
        SampleFormat::F32 => Some(0),
        SampleFormat::I16 => Some(1),
        _ => None,
    }
}

/// The period. The sound server's own default is a few ms, which wakes the callback hundreds of times a
/// second for music buffered seconds ahead. It stays this while the equalizer is tuned: the engine's
/// ring goes shallow ([`AudioOutput::shallow`] is left as nothing here), and a stream rebuilt for a
/// shorter period was a gap in the music on entering and leaving the equalizer.
const PERIOD_MS: u32 = 100;

fn buffer_size(rate: u32, ms: u32, periods: Option<(u32, u32)>) -> BufferSize {
    match periods {
        Some((min, max)) => BufferSize::Fixed((rate * ms / 1000).clamp(min, max)),
        None => BufferSize::Default,
    }
}

/// Builds a paused stream that pulls `feed`, scales by `volume` and records when it is heard. A contended
/// feed lock (only possible while a replaced stream is still stopping) plays silence.
#[allow(clippy::too_many_arguments)]
fn build_stream<T: SizedSample + Default + Send + 'static>(
    device: &Device,
    config: StreamConfig,
    feed: Arc<Mutex<Feed>>,
    volume: Volume,
    heard: Arc<Heard>,
    on_error: impl FnMut(cpal::Error) + Send + 'static,
    pull: fn(&mut Feed, &mut [T]) -> usize,
    scale: fn(T, f32) -> T,
) -> Result<Stream, String> {
    let rate = config.sample_rate;
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                let Some(mut feed) = feed.try_lock() else {
                    data.fill(T::default());
                    return;
                };
                let music = pull(&mut feed, data);
                let t = info.timestamp();
                let delay = t.playback.saturating_duration_since(t.callback).as_micros() as u64;
                heard.pulled(clock_us() + delay, music, rate);
                // A flush wakes the engine before its discarded frames are accounted for here.
                if feed.flushed() {
                    feed.wake_engine();
                }
                drop(feed);
                let v = volume.get();
                if v != 1.0 {
                    data.iter_mut().for_each(|s| *s = scale(*s, v));
                }
            },
            on_error,
            None,
        )
        .map_err(|e| e.to_string())?;
    let _ = stream.pause();
    Ok(stream)
}

/// A stream error other than a device change: one whose device is gone stops the stream for good, so the
/// engine is told ([`AudioOutput::failed`]) and woken to hear it; others are only logged.
fn stream_failed(kind: ErrorKind, detail: String, failed: &Mutex<Option<String>>, wake: impl FnOnce()) {
    let _ = std::io::Write::write_fmt(&mut std::io::stderr(), format_args!("nori: the output stream failed: {detail}\n"));
    if matches!(kind, ErrorKind::DeviceNotAvailable | ErrorKind::HostUnavailable | ErrorKind::StreamInvalidated) {
        *failed.lock() = Some(detail);
        wake();
    }
}

impl CpalOutput {
    /// The stream stopped for good: the engine is told ([`AudioOutput::failed`]) and woken to hear it.
    fn fail(&self, why: String) {
        *self.failed.lock() = Some(why);
        if let Some(feed) = &self.feed {
            feed.lock().wake_engine();
        }
    }

    /// Drops the current stream, then builds a new one from the current config and feed, playing if
    /// the engine wants it playing.
    fn rebuild(&mut self) -> Result<(), String> {
        self.stream = None;
        let (Some(device), Some((config, format)), Some(feed)) = (&self.device, &self.config, &self.feed) else { return Err("not open".into()) };
        // Only follows the default device; a named device stays put.
        let watch = self.watch.clone().filter(|_| self.wanted.is_none());
        let (failed, stopped) = (self.failed.clone(), feed.clone());
        let on_error = move |e: cpal::Error| match e.kind() {
            ErrorKind::DeviceChanged => {
                if let (Some(w), Some(d)) = (&watch, cpal::default_host().default_output_device().as_ref().and_then(describe)) {
                    w(d);
                }
            }
            kind => stream_failed(kind, e.to_string(), &failed, || stopped.lock().wake_engine()),
        };
        let (feed, volume, heard) = (feed.clone(), self.volume.clone(), self.heard.clone());
        let stream = match format {
            SampleFormat::F32 => build_stream(device, *config, feed, volume, heard, on_error, Feed::pull, |s, v| s * v),
            _ => build_stream(device, *config, feed, volume, heard, on_error, Feed::pull_i16, |s, v| (s as f32 * v) as i16),
        }?;
        if self.playing {
            stream.play().map_err(|e| e.to_string())?;
        }
        self.stream = Some(stream);
        Ok(())
    }
}

impl AudioOutput for CpalOutput {
    fn open(&mut self, want: OutputFormat) -> Result<OutputFormat, String> {
        let device = self.pick()?;
        let exact = device
            .supported_output_configs()
            .map_err(|e| e.to_string())?
            .filter(|r| r.channels() as usize == want.channels && rank(r.sample_format()).is_some())
            .filter_map(|r| r.try_with_sample_rate(want.rate))
            .min_by_key(|c| rank(c.sample_format()));
        let chosen = match exact {
            Some(c) => c,
            None => device.default_output_config().map_err(|e| e.to_string())?,
        };
        let format = chosen.sample_format();
        if rank(format).is_none() {
            return Err(format!("the device only takes {format:?} samples"));
        }
        self.periods = match *chosen.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } if max >= min => Some((min, max)),
            _ => None,
        };
        let buffer_size = buffer_size(chosen.sample_rate(), PERIOD_MS, self.periods);
        let config = StreamConfig { channels: chosen.channels(), sample_rate: chosen.sample_rate(), buffer_size };
        let got = OutputFormat { rate: config.sample_rate, channels: config.channels as usize, bits: 0 };
        if let (Some(w), Some(d)) = (&self.watch, describe(&device)) {
            w(d);
        }
        self.device = Some(device);
        self.config = Some((config, format));
        Ok(got)
    }

    fn start(&mut self, feed: Feed) -> Result<(), String> {
        self.heard.follow(feed.pulled());
        self.feed = Some(Arc::new(Mutex::new(feed)));
        self.rebuild()
    }

    fn pause(&mut self) {
        self.playing = false;
        if let Some(s) = &self.stream {
            let _ = s.pause();
        }
    }

    fn resume(&mut self) {
        self.playing = true;
        if let Some(s) = &self.stream {
            if let Err(e) = s.play() {
                self.fail(e.to_string());
            }
        }
    }

    fn watch(&mut self, changed: DeviceWatch) {
        self.watch = Some(Arc::new(changed));
    }

    fn latency_us(&self) -> u64 {
        self.heard.left_us(clock_us())
    }

    fn holding(&self) -> bool {
        self.latency_us() > 0
    }

    fn failed(&mut self) -> Option<String> {
        self.failed.lock().take()
    }

    fn takes_float(&mut self) -> bool {
        let Ok(device) = self.pick() else { return false };
        device.supported_output_configs().is_ok_and(|mut c| c.any(|r| r.sample_format() == SampleFormat::F32))
    }

    fn close(&mut self) {
        self.stream = None;
        self.feed = None;
        self.device = None;
        self.playing = false;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_default_sink_names_what_plays() {
        const SINKS: &str = "Sink #65\n\tState: RUNNING\n\tName: alsa_output.usb-Realtek_HECATE_G30_S-00.analog-stereo\n\tDescription: HECATE G30 S  Analog Stereo\n\nSink #88\n\tName: bluez_output.58_18_62_25_85_CB.1\n\tDescription: WH-1000XM6\n\nSink #90\n\tName: alsa_output.pci-0000_07_00.1.hdmi-stereo\n\tDescription: GM206 High Definition Audio Controller Digital Stereo (HDMI)\n";
        for (name, kind, words) in [
            ("alsa_output.usb-Realtek_HECATE_G30_S-00.analog-stereo", OutputKind::Usb, "HECATE G30 S"),
            ("bluez_output.58_18_62_25_85_CB.1", OutputKind::Bluetooth, "WH-1000XM6"),
            ("alsa_output.pci-0000_07_00.1.hdmi-stereo", OutputKind::Line, "GM206 High Definition Audio Controller"),
        ] {
            let d = sink(SINKS, name).unwrap();
            assert_eq!((d.kind, d.name.as_str()), (kind, words), "{name}");
        }
        assert!(sink(SINKS, "gone").is_none());
    }

    use super::*;

    #[test]
    fn a_device_gone_stops_the_engine() {
        let failed = Mutex::new(None);
        for (kind, stops) in [(ErrorKind::DeviceNotAvailable, true), (ErrorKind::HostUnavailable, true), (ErrorKind::DeviceBusy, false), (ErrorKind::StreamInvalidated, true)] {
            let mut woke = false;
            stream_failed(kind, "gone".into(), &failed, || woke = true);
            assert_eq!((failed.lock().take().is_some(), woke), (stops, stops), "{kind:?}");
        }
    }
}
