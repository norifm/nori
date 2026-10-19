//! The visionOS player answer: which audio to play, at which URL, for how long.

use std::time::Duration;

use serde_json::Value;

/// One audio format of a video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Format {
    pub itag: u32,
    pub url: String,
    /// "audio/webm; codecs=\"opus\"", ...
    pub mime: String,
    pub bit_rate: u32,
    /// Bytes; None when YouTube does not say.
    pub length: Option<u64>,
}

impl Format {
    fn opus(&self) -> bool {
        self.mime.starts_with("audio/webm") && self.mime.contains("opus")
    }

    fn aac(&self) -> bool {
        self.mime.starts_with("audio/mp4")
    }
}

/// The format picked to play a video, and how long YouTube serves its URL from when it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stream {
    pub format: Format,
    pub expires_in: Duration,
}

/// The most bytes one request of a stream's URL asks for (a `Range` header with an end): YouTube sends
/// a longer range, or one without an end, at about the speed it plays.
pub const PART: u64 = 10 * 1024 * 1024;

/// Opus 160 kbps (itag 251), else AAC 128 kbps (140), else the richest Opus, else the richest AAC.
pub fn pick(formats: &[Format]) -> Option<&Format> {
    let richest = |f: fn(&Format) -> bool| formats.iter().filter(|x| f(x)).max_by_key(|x| x.bit_rate);
    formats.iter().find(|f| f.itag == 251).or_else(|| formats.iter().find(|f| f.itag == 140)).or_else(|| richest(Format::opus)).or_else(|| richest(Format::aac))
}

/// Whether a player answer refuses for want of a visitor id ("Sign in to confirm you're not a bot").
pub(crate) fn wants_visitor(v: &Value) -> bool {
    v.pointer("/playabilityStatus/status").and_then(Value::as_str) == Some("LOGIN_REQUIRED")
}

/// Served URLs last this long when YouTube does not say (its usual 6 hours, less a margin).
const USUAL_EXPIRY: Duration = Duration::from_secs(5 * 3600);

/// Why a player answer gives no stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum NoStream {
    /// YouTube will not play the video; its reason.
    Refused(String),
    /// It would, but offers no plain audio URL.
    NoAudio,
}

/// The stream of a player answer.
pub(crate) fn stream(v: &Value) -> Result<Stream, NoStream> {
    let status = v.pointer("/playabilityStatus/status").and_then(Value::as_str).unwrap_or("");
    if status != "OK" {
        let reason = v.pointer("/playabilityStatus/reason").and_then(Value::as_str).unwrap_or(status);
        return Err(NoStream::Refused(reason.to_string()));
    }
    let formats: Vec<Format> = v
        .pointer("/streamingData/adaptiveFormats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|f| {
            let mime = f.get("mimeType")?.as_str()?;
            Some(Format {
                itag: f.get("itag")?.as_u64()? as u32,
                // A format without a plain URL is ciphered (another client's answer): not playable here.
                url: f.get("url")?.as_str()?.to_string(),
                mime: mime.to_string(),
                bit_rate: f.get("bitrate").and_then(Value::as_u64).unwrap_or(0) as u32,
                length: f.get("contentLength").and_then(Value::as_str).and_then(|l| l.parse().ok()),
            })
        })
        .filter(|f| f.mime.starts_with("audio/"))
        .collect();
    let format = pick(&formats).cloned().ok_or(NoStream::NoAudio)?;
    let expires_in = v.pointer("/streamingData/expiresInSeconds").and_then(Value::as_str).and_then(|s| s.parse().ok()).map_or(USUAL_EXPIRY, Duration::from_secs);
    Ok(Stream { format, expires_in })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;

    fn answer(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).unwrap()
    }

    fn f(itag: u32, mime: &str, bit_rate: u32) -> Format {
        Format { itag, url: format!("u{itag}"), mime: mime.into(), bit_rate, length: None }
    }

    #[test]
    fn a_playable_answer_gives_opus_251() {
        let s = stream(&answer("player")).unwrap();
        assert_eq!((s.format.itag, s.format.length, s.expires_in), (251, Some(3_433_755), Duration::from_secs(21_540)));
        assert!(s.format.url.starts_with("https://rr1---sn-fixture.googlevideo.com/videoplayback?") && s.format.url.contains("itag=251"));
        assert!(s.format.opus());
    }

    #[test]
    fn an_unplayable_answer_says_why() {
        assert_eq!(stream(&answer("unplayable")), Err(NoStream::Refused("This video is unavailable".into())));
    }

    /// Ciphered formats (no plain `url`) and video formats are never picked.
    #[test]
    fn only_plain_audio_is_picked() {
        let mut v = answer("player");
        for f in v.pointer_mut("/streamingData/adaptiveFormats").unwrap().as_array_mut().unwrap() {
            if f["itag"] == 251 || f["itag"] == 140 {
                f.as_object_mut().unwrap().remove("url");
            }
        }
        assert_eq!(stream(&v).unwrap().format.itag, 249, "the Opus left");
        v["streamingData"]["adaptiveFormats"].as_array_mut().unwrap().retain(|f| f["itag"] == 137);
        assert_eq!(stream(&v), Err(NoStream::NoAudio), "video only");
    }

    #[test]
    fn picks() {
        let opus = "audio/webm; codecs=\"opus\"";
        let aac = "audio/mp4; codecs=\"mp4a.40.2\"";
        let all = [f(249, opus, 50), f(140, aac, 130), f(251, opus, 137), f(250, opus, 65)];
        assert_eq!(pick(&all).map(|f| f.itag), Some(251));
        assert_eq!(pick(&all[..2]).map(|f| f.itag), Some(140), "AAC 140 before a poorer Opus");
        assert_eq!(pick(&[f(249, opus, 50), f(250, opus, 65)]).map(|f| f.itag), Some(250));
        assert_eq!(pick(&[f(139, aac, 48), f(141, aac, 256)]).map(|f| f.itag), Some(141));
        assert_eq!(pick(&[]), None);
    }
}
