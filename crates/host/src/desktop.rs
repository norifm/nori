//! What the terminal and desktop clients share around their session: the app's queue session over its
//! settings, their own settings kept as app values, logging in, the sound card (cpal) and the session
//! opened over them.

use std::path::Path;
use std::sync::Arc;

use nori_core::remote::wire::DeviceKind;
use nori_core::settings::SavedServer;
use nori_core::transport::{block_on, NetError};
use nori_engine::AudioOutput;
use nori_http::Http;
use nori_output_cpal::CpalOutput;

use crate::session::{Open, Out, Session};
use crate::Level;

/// A calendar date, and whether the zone it is in lies south of the equator (the seasons turn half a year
/// later there).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Today {
    pub month: u32,
    pub day: u32,
    pub southern: bool,
}

impl Today {
    /// The local date now, in the local time zone.
    pub fn now() -> Today {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let (month, day) = nori_look::theme::utc_date(now + nori_core::library::local_offset_s(now));
        let link = std::fs::read_link("/etc/localtime").map(|p| p.to_string_lossy().into_owned()).ok();
        Today { month, day, southern: zone(std::env::var("TZ").ok(), link).is_some_and(|z| nori_look::theme::southern(&z)) }
    }
}

/// The IANA time zone: `tz` (the `TZ` variable), else where `link` (`/etc/localtime`) points.
fn zone(tz: Option<String>, link: Option<String>) -> Option<String> {
    if let Some(tz) = tz.filter(|z| !z.is_empty()) {
        return Some(tz.trim_start_matches(':').to_string());
    }
    link?.split_once("zoneinfo/").map(|(_, z)| z.to_string())
}

/// The app's queue session over its settings: the client's object graph root, which every view reads.
/// Global: the root of the client's own object graph (as Kotlin's `Nori`), made once a process.
pub fn app() -> &'static Arc<nori_core::queue::Session> {
    static APP: std::sync::OnceLock<Arc<nori_core::queue::Session>> = std::sync::OnceLock::new();
    APP.get_or_init(|| Arc::new(nori_core::queue::Session::new(nori_core::settings_store::Settings::new())))
}

/// The client's own settings, kept as app values in the database under keys of its own.
pub mod own {
    pub fn text(key: &str) -> Option<String> {
        super::app().settings.app_value(key).filter(|v| !v.is_empty())
    }

    pub fn flag(key: &str, default: bool) -> bool {
        super::app().settings.app_value(key).map_or(default, |v| v == "true")
    }

    pub fn number(key: &str, default: f32) -> f32 {
        super::app().settings.app_value(key).and_then(|v| v.parse().ok()).unwrap_or(default)
    }

    pub fn keep(key: &'static str, value: String) {
        super::app().settings.keep_app_value(key, value);
    }
}

/// Logs `draft` in and keeps it as the active profile, blocking ([`nori_core::providers::log_in`]).
pub fn log_in(http: Arc<Http>, draft: SavedServer) -> Result<SavedServer, NetError> {
    block_on(nori_core::providers::log_in(http, &app().settings, draft))
}

/// The cpal device named `device` (else the system's default) at `volume` (0 to 1).
fn sound(device: Option<&str>, volume: f32) -> (Box<dyn AudioOutput>, Arc<Level>) {
    let card = match device {
        Some(name) => CpalOutput::with_device(name),
        None => CpalOutput::new(),
    };
    let level = card.volume();
    (Box::new(card), Level::new(volume, Some(Box::new(move |v| level.set(v)))))
}

/// A desktop session's opening.
pub struct DesktopOpen<'a> {
    pub data: &'a Path,
    pub http: Arc<Http>,
    pub profile: SavedServer,
    /// The sound card by name; None for the system's default.
    pub device: Option<String>,
    /// 0 to 1.
    pub volume: f32,
    pub covers: Option<nori_covers::loader::Config>,
    pub offline: bool,
    /// The process's media controls, driven by this session while it is open.
    pub mpris: Option<Arc<nori_mpris::Mpris>>,
    /// This client, as the account's other devices list it.
    pub kind: DeviceKind,
    pub out: Out,
}

/// Opens the profile on the sound card, without touching the network ([`Session::open`]).
pub fn open(o: DesktopOpen) -> Result<Session, String> {
    let (output, volume) = sound(o.device.as_deref(), o.volume);
    Session::open(Open {
        queue: app().clone(),
        data: o.data,
        http: o.http,
        profile: o.profile,
        output,
        volume,
        memory_mb: 256,
        covers: o.covers,
        offline: o.offline,
        mpris: o.mpris,
        device: nori_core::remote::RemoteMe { name: crate::device_name(), kind: o.kind },
        discovery: None,
        tasks: crate::tasks(),
        keeps_music: true,
        decoders: Default::default(),
        out: o.out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_zone_is_tz_else_the_localtime_link() {
        let link = || Some("/usr/share/zoneinfo/Australia/Sydney".to_string());
        assert_eq!(zone(None, link()).as_deref(), Some("Australia/Sydney"));
        assert_eq!(zone(Some(":Europe/Berlin".into()), link()).as_deref(), Some("Europe/Berlin"));
        assert_eq!(zone(Some(String::new()), link()).as_deref(), Some("Australia/Sydney"), "an empty TZ is unset");
        assert_eq!(zone(None, Some("/etc/UTC".into())), None);
    }
}
