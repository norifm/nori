//! What a scan reads of a music file, through symphonia's readers (the engine's, plus the ID3v2, ID3v1
//! and APE tag readers on a probe of this module's own): its tags, its audio's rate, channels and
//! length, whether it holds a picture, and a fingerprint of its audio. A file with no audio nori
//! decodes is left out.

use std::sync::OnceLock;

use nori_model::ReplayGain;
use nori_player::decode::Codec;
use serde::{Deserialize, Serialize};
use symphonia::core::codecs::audio::well_known::{CODEC_ID_PCM_F32LE, CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S24LE, CODEC_ID_PCM_S32LE, CODEC_ID_PCM_U8};
use symphonia::core::formats::probe::{Hint, Probe};
use symphonia::core::formats::{Attachment, FormatOptions, FormatReader, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::{MetadataOptions, MetadataRevision, StandardTag, StandardVisualKey, Visual};
use symphonia_metadata::{ape::ApeReader, id3v1::Id3v1Reader, id3v2::Id3v2Reader};

use crate::Fnv;

/// The tags of a music file and the facts of its audio, as a scan keeps them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tags {
    pub title: Option<String>,
    /// Each artist credited, in order.
    pub artists: Vec<String>,
    pub album_artist: Option<String>,
    pub album: Option<String>,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub year: Option<u32>,
    pub genre: Option<String>,
    /// Tagged as part of a compilation.
    pub compilation: bool,
    pub gain: Option<ReplayGain>,
    /// The MusicBrainz recording.
    pub recording: Option<String>,
    pub bpm: Option<u32>,
    pub comment: Option<String>,
    pub duration_ms: u64,
    pub sample_rate: u32,
    pub channels: u32,
    pub bit_depth: Option<u32>,
    /// It holds a picture ([`picture`]).
    pub picture: bool,
}

/// What a scan read of a music file.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub tags: Tags,
    /// A hash of its first audio packets and its audio's facts: the same for the same audio whatever its
    /// tags, name or folder, so a song is known again after it moved ([`crate::scan::rematch`]).
    pub fingerprint: u64,
}

/// Why a file was left out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Left {
    /// No container nori reads, or a broken one.
    Unread,
    /// No audio track, or one in a codec nori does not decode.
    NotPlayable,
}

/// Bytes of audio packets the fingerprint hashes: a few seconds of any codec.
const FINGERPRINT_BYTES: usize = 128 << 10;

/// The probe: the engine's container readers and the tag readers around them. Global: it is fixed
/// once made and symphonia's own is kept the same way.
fn probe() -> &'static Probe {
    static PROBE: OnceLock<Probe> = OnceLock::new();
    PROBE.get_or_init(|| {
        let mut p = Probe::new();
        symphonia::default::register_enabled_formats(&mut p);
        p.register_metadata::<Id3v2Reader<'_>>();
        p.register_metadata::<Id3v1Reader<'_>>();
        p.register_metadata::<ApeReader<'_>>();
        p
    })
}

fn open(source: Box<dyn MediaSource>, extension: &str) -> Result<Box<dyn FormatReader>, Left> {
    let mss = MediaSourceStream::new(source, MediaSourceStreamOptions::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    probe().probe(&hint, mss, FormatOptions::default(), MetadataOptions::default()).map_err(|_| Left::Unread)
}

/// Whether nori decodes a track of `codec`: by its own decoders, or as WAV's PCM.
fn decoded(codec: symphonia::core::codecs::audio::AudioCodecId) -> bool {
    Codec::of(codec).is_some() || [CODEC_ID_PCM_U8, CODEC_ID_PCM_S16LE, CODEC_ID_PCM_S24LE, CODEC_ID_PCM_S32LE, CODEC_ID_PCM_F32LE].contains(&codec)
}

/// Reads the file in `source`, whose extension is `extension`.
pub fn read(source: Box<dyn MediaSource>, extension: &str) -> Result<Read, Left> {
    let mut reader = open(source, extension)?;
    let track = reader.default_track(TrackType::Audio).ok_or(Left::NotPlayable)?;
    let params = track.codec_params.as_ref().and_then(|p| p.audio()).ok_or(Left::NotPlayable)?;
    if !decoded(params.codec) {
        return Err(Left::NotPlayable);
    }
    let (id, sample_rate) = (track.id, params.sample_rate.ok_or(Left::NotPlayable)?);
    let channels = params.channels.as_ref().map_or(0, |c| c.count() as u32);
    let bit_depth = params.bits_per_sample.or(params.bits_per_coded_sample);
    let mut fingerprint = Fnv::default();
    fingerprint.add(params.codec.to_string().as_bytes());
    fingerprint.add(&sample_rate.to_le_bytes());
    fingerprint.add(&channels.to_le_bytes());
    let stated_ms = match (track.num_frames, track.time_base, track.duration) {
        (Some(frames), _, _) => Some(frames * 1000 / sample_rate as u64),
        (None, Some(base), Some(d)) => base.calc_duration(d).map(|t| t.as_millis().max(0) as u64),
        _ => None,
    };
    let base = track.time_base;
    let mut tags = Tags { sample_rate, channels, bit_depth, ..Tags::default() };
    while let Some(revision) = reader.metadata().pop() {
        take(&mut tags, &revision);
    }
    if let Some(revision) = reader.metadata().current() {
        take(&mut tags, revision);
    }
    tags.picture |= reader.attachments().iter().any(|a| image_of(a).is_some());
    // The fingerprint's packets, and with no length stated, every packet's length added up.
    let (mut hashed, mut ticks) = (0, 0u64);
    while hashed < FINGERPRINT_BYTES || stated_ms.is_none() {
        let Ok(Some(packet)) = reader.next_packet() else { break };
        if packet.track_id != id {
            continue;
        }
        if hashed < FINGERPRINT_BYTES {
            fingerprint.add(&packet.data);
            hashed += packet.data.len();
        }
        ticks += packet.dur.get();
    }
    if hashed == 0 {
        return Err(Left::NotPlayable);
    }
    tags.duration_ms = stated_ms.unwrap_or_else(|| base.and_then(|b| b.calc_duration(ticks.into())).map_or(0, |t| t.as_millis().max(0) as u64));
    Ok(Read { tags, fingerprint: fingerprint.0 })
}

/// The picture in the file in `source`: its front cover, else the first it holds; None without one.
pub fn picture(source: Box<dyn MediaSource>, extension: &str) -> Option<Vec<u8>> {
    let mut reader = open(source, extension).ok()?;
    let mut visuals: Vec<Visual> = Vec::new();
    while let Some(revision) = reader.metadata().pop() {
        visuals.extend(revision.media.visuals);
    }
    if let Some(revision) = reader.metadata().current() {
        visuals.extend(revision.media.visuals.iter().cloned());
    }
    let front = visuals.iter().position(|v| v.usage == Some(StandardVisualKey::FrontCover)).unwrap_or(0);
    match visuals.into_iter().nth(front) {
        Some(v) => Some(v.data.into_vec()),
        None => reader.attachments().iter().find_map(image_of).map(<[u8]>::to_vec),
    }
}

/// The picture a container attaches (a Matroska file's cover).
fn image_of(a: &Attachment) -> Option<&[u8]> {
    match a {
        Attachment::File(f) if f.media_type.as_deref().is_some_and(|t| t.starts_with("image/")) => Some(&f.data),
        _ => None,
    }
}

/// `tags` with what `revision` says; a later revision's tag stands over an earlier one's.
fn take(tags: &mut Tags, revision: &MetadataRevision) {
    let text = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
    let mut artists = Vec::new();
    for tag in &revision.media.tags {
        let Some(std) = &tag.std else { continue };
        match std {
            StandardTag::TrackTitle(s) => tags.title = text(s).or(tags.title.take()),
            StandardTag::Artist(s) => artists.extend(text(s)),
            StandardTag::AlbumArtist(s) => tags.album_artist = text(s).or(tags.album_artist.take()),
            StandardTag::Album(s) => tags.album = text(s).or(tags.album.take()),
            StandardTag::TrackNumber(n) => tags.track = Some(*n as u32),
            StandardTag::DiscNumber(n) => tags.disc = Some(*n as u32),
            StandardTag::RecordingYear(y) | StandardTag::ReleaseYear(y) | StandardTag::OriginalReleaseYear(y) => tags.year = tags.year.or(Some(*y as u32)),
            StandardTag::RecordingDate(s) | StandardTag::ReleaseDate(s) => tags.year = tags.year.or_else(|| year_of(s)),
            StandardTag::Genre(s) => tags.genre = tags.genre.take().or_else(|| text(s)),
            StandardTag::CompilationFlag(on) => tags.compilation = *on,
            StandardTag::ReplayGainTrackGain(s) => gain(tags).track_gain = decibels(s),
            StandardTag::ReplayGainTrackPeak(s) => gain(tags).track_peak = decibels(s),
            StandardTag::ReplayGainAlbumGain(s) => gain(tags).album_gain = decibels(s),
            StandardTag::ReplayGainAlbumPeak(s) => gain(tags).album_peak = decibels(s),
            StandardTag::MusicBrainzRecordingId(s) => tags.recording = text(s),
            StandardTag::Bpm(n) => tags.bpm = Some(*n as u32),
            StandardTag::Comment(s) => tags.comment = tags.comment.take().or_else(|| text(s)),
            _ => {}
        }
    }
    if !artists.is_empty() {
        artists.dedup();
        tags.artists = artists;
    }
    tags.picture |= !revision.media.visuals.is_empty();
}

fn gain(tags: &mut Tags) -> &mut ReplayGain {
    tags.gain.get_or_insert_with(ReplayGain::default)
}

/// A ReplayGain value as tagged ("-6.20 dB", "0.988"); None for one that is no number.
fn decibels(s: &str) -> Option<f32> {
    s.trim().trim_end_matches(|c: char| c.is_alphabetic() || c.is_whitespace()).trim().parse().ok()
}

/// The year a tagged date starts with ("1997-05-21", "1997").
fn year_of(s: &str) -> Option<u32> {
    let digits: String = s.trim().chars().take_while(char::is_ascii_digit).collect();
    (digits.len() == 4).then(|| digits.parse().ok()).flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::flac;
    use std::io::Cursor;

    fn read_flac(bytes: Vec<u8>) -> Result<Read, Left> {
        read(Box::new(Cursor::new(bytes)), "flac")
    }

    #[test]
    fn a_flac_file_reads_its_tags_length_and_picture() {
        let comments = ["TITLE=Opening", "ARTIST=Ann", "ARTIST=Bo", "ALBUMARTIST=Ann", "ALBUM=First", "TRACKNUMBER=3", "DISCNUMBER=2", "DATE=1997-05-21", "GENRE=Jazz", "COMPILATION=1", "REPLAYGAIN_TRACK_GAIN=-6.20 dB", "REPLAYGAIN_TRACK_PEAK=0.988"];
        let r = read_flac(flac(440.0, &comments, Some(b"\x89PNG"))).unwrap();
        let t = &r.tags;
        assert_eq!((t.title.as_deref(), t.artists.as_slice(), t.album_artist.as_deref(), t.album.as_deref()), (Some("Opening"), &["Ann".to_string(), "Bo".to_string()][..], Some("Ann"), Some("First")));
        assert_eq!((t.track, t.disc, t.year, t.genre.as_deref(), t.compilation), (Some(3), Some(2), Some(1997), Some("Jazz"), true));
        assert_eq!(t.gain.as_ref().map(|g| (g.track_gain, g.track_peak)), Some((Some(-6.2), Some(0.988))));
        assert_eq!((t.duration_ms, t.sample_rate, t.channels, t.picture), (1000, 8000, 2, true));
        assert_eq!(picture(Box::new(Cursor::new(flac(440.0, &[], Some(b"\x89PNG")))), "flac").as_deref(), Some(&b"\x89PNG"[..]));
        assert_eq!(picture(Box::new(Cursor::new(flac(440.0, &[], None))), "flac"), None);
    }

    /// The fingerprint follows the audio alone: the same audio tagged anew keeps it, other audio has its own.
    #[test]
    fn the_fingerprint_is_the_audio_not_the_tags() {
        let a = read_flac(flac(440.0, &["TITLE=One"], None)).unwrap().fingerprint;
        assert_eq!(read_flac(flac(440.0, &["TITLE=Renamed", "ARTIST=Someone"], Some(b"pic"))).unwrap().fingerprint, a);
        assert_ne!(read_flac(flac(660.0, &["TITLE=One"], None)).unwrap().fingerprint, a);
    }

    #[test]
    fn a_file_without_audio_nori_plays_is_left_out() {
        assert_eq!(read(Box::new(Cursor::new(b"not music at all".repeat(100))), "mp3"), Err(Left::Unread));
        let mut wav = nori_player::files::wav(8000, &nori_player::files::sine(8000, 440.0, 0.5, 8000.0));
        // An A-law WAV (format 6): a container nori reads, a codec it does not decode.
        wav[20] = 6;
        assert!(read(Box::new(Cursor::new(wav)), "wav").is_err());
        let r = read(Box::new(Cursor::new(nori_player::files::wav(8000, &nori_player::files::sine(8000, 440.0, 0.5, 8000.0)))), "wav").unwrap();
        assert_eq!((r.tags.duration_ms, r.tags.title), (500, None), "a WAV plays, untagged");
    }

    #[test]
    fn tagged_numbers_read_as_numbers() {
        assert_eq!([decibels("-6.20 dB"), decibels("+1.5"), decibels("loud")], [Some(-6.2), Some(1.5), None]);
        assert_eq!([year_of("1997-05-21"), year_of("2003"), year_of("97"), year_of("")], [Some(1997), Some(2003), None, None]);
    }
}
