//! The LAN and casting interfaces a plugin calls (`discovery`, `lan-serve`, `media`, `audio-out`,
//! `cast`), over the host's [`Lan`](crate::lan::Lan), and what a client hands the plugins of them.

use std::sync::{Arc, Weak};

use wasmtime::component::Resource;

use crate::lan::{Cast, CastButton, CastKind};
use crate::runtime::nori::plugin::{audio_out, cast, discovery, lan_serve, media};
use crate::runtime::points::hooks::Event;
use crate::runtime::{types, Host};
use crate::sink::{Encoding, Sink};
use crate::sockets::{error, Tcp};
use crate::Plugins;

/// What makes a song's URLs for another device: the open profile.
pub trait Media: Send + Sync {
    /// Song `id`'s stream at its own quality.
    fn stream_url(&self, id: &str) -> Option<String>;
    fn cover_url(&self, id: &str, size: u32) -> Option<String>;
}

/// Where nori's music plays while a plugin casts it.
pub enum Card {
    /// Its audio is pumped into this sink instead of the device's own output.
    Sink(Arc<Sink>),
    /// On the device's own output again.
    Here,
    /// The cast's device plays the songs itself: the player here lets its output go.
    Elsewhere,
}

/// What a client does with each [`Card`]: its engine's.
pub type Cards = Box<dyn Fn(Card) + Send + Sync>;

/// A service mDNS found.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Service {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub txt: Vec<TxtEntry>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct TxtEntry {
    pub key: String,
    pub value: String,
}

fn needs_lan(h: &Host) -> Result<(), types::Error> {
    if h.manifest.lan() {
        Ok(())
    } else {
        Err(error(types::ErrorKind::Refused, "needs lan"))
    }
}

impl discovery::Host for Host {
    fn browse(&mut self, kind: String) {
        match needs_lan(self) {
            Ok(()) => self.lan.browse(&self.id, &kind),
            Err(e) => nori_model::alog::info(&format!("plugin {}: no browsing: {}", self.id, e.detail)),
        }
    }

    fn stop(&mut self, kind: String) {
        self.lan.stop_browsing(&self.id, &kind);
    }
}

impl lan_serve::Host for Host {
    fn base(&mut self) -> Option<String> {
        self.lan.base().map(|b| format!("{b}/p/{}", self.id))
    }

    fn open(&mut self) {
        match needs_lan(self) {
            Ok(()) => self.lan.serve(&self.id, true),
            Err(e) => nori_model::alog::info(&format!("plugin {}: no door: {}", self.id, e.detail)),
        }
    }

    fn close(&mut self) {
        self.lan.serve(&self.id, false);
    }
}

impl Host {
    /// `url` as another device on the LAN reaches it: as it is, or through the door when it is this
    /// device's own (a server on localhost).
    fn reachable_url(&self, url: String) -> Result<String, types::Error> {
        let host = crate::host::host_of(&url).ok_or_else(|| error(types::ErrorKind::Other, "not an http(s) URL"))?;
        let here = host.eq_ignore_ascii_case("localhost") || host.parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback());
        if !here {
            return Ok(url);
        }
        if !url.starts_with("http://") {
            return Err(error(types::ErrorKind::Unsupported, "only plain HTTP is proxied"));
        }
        self.lan.proxy(&self.id, url).ok_or_else(|| error(types::ErrorKind::Network, "not on a LAN"))
    }

    fn media(&self) -> Result<Arc<dyn Media>, types::Error> {
        self.lan.media().ok_or_else(|| error(types::ErrorKind::Unsupported, "no profile open"))
    }
}

impl media::Host for Host {
    fn stream_url(&mut self, id: String) -> Result<String, types::Error> {
        let url = self.media()?.stream_url(&id).ok_or_else(|| error(types::ErrorKind::NotFound, format!("no stream for {id}")))?;
        self.reachable_url(url)
    }

    fn cover_url(&mut self, id: String, size: u32) -> Result<String, types::Error> {
        let url = self.media()?.cover_url(&id, size).ok_or_else(|| error(types::ErrorKind::NotFound, format!("no cover for {id}")))?;
        self.reachable_url(url)
    }
}

fn encoding(e: audio_out::Encoding) -> Encoding {
    match e {
        audio_out::Encoding::Pcm16 => Encoding::Pcm16,
        audio_out::Encoding::Wav => Encoding::Wav,
        audio_out::Encoding::Flac => Encoding::Flac,
    }
}

impl audio_out::Host for Host {
    fn to_socket(&mut self, socket: Resource<Tcp>, e: audio_out::Encoding, rate: u32, channels: u8) -> Result<(), types::Error> {
        if !(8_000..=192_000).contains(&rate) || !(1..=8).contains(&channels) {
            return Err(error(types::ErrorKind::Refused, format!("{rate} Hz, {channels} channels")));
        }
        let tcp = self.table.get(&socket).map_err(|e| error(types::ErrorKind::Other, e.to_string()))?;
        let plain = tcp.plain().ok_or_else(|| error(types::ErrorKind::Unsupported, "audio over TLS"))?;
        let stream = plain.try_clone().map_err(|e| error(types::ErrorKind::Network, e.to_string()))?;
        self.lan.pump_sink(&self.id, Arc::new(Sink::to_socket(encoding(e), stream, rate, channels)));
        Ok(())
    }

    fn serve(&mut self, e: audio_out::Encoding) -> Result<String, types::Error> {
        needs_lan(self)?;
        self.lan.serve_sink(&self.id, Arc::new(Sink::served(encoding(e)))).ok_or_else(|| error(types::ErrorKind::Network, "not on a LAN"))
    }

    fn set_latency(&mut self, ms: u32) {
        self.lan.sink_latency(&self.id, ms);
    }

    fn stop(&mut self) {
        self.lan.stop_sink(&self.id);
    }
}

impl cast::Host for Host {
    fn start(&mut self, kind: cast::Kind, device_name: String) {
        let kind = match kind {
            cast::Kind::Sink => CastKind::Sink,
            cast::Kind::Target => CastKind::Target,
        };
        self.lan.start_cast(&self.id, kind, device_name);
    }

    fn report(&mut self, playing: bool, position_ms: u64, volume: Option<u8>) {
        self.lan.report_cast(&self.id, playing, position_ms, volume.map(|v| v.min(100)));
    }

    fn end(&mut self) {
        self.lan.end_cast(&self.id);
    }
}

impl Plugins {
    /// What makes songs' URLs for plugins (`media`): the open profile; held weakly.
    pub fn set_media(&self, media: Weak<dyn Media>) {
        self.lan.set_media(Some(media));
    }

    /// Where nori's audio goes while a plugin casts it ([`Cards`]); the client's engine.
    pub fn set_cards(&self, cards: Cards) {
        self.lan.set_cards(cards);
    }

    /// Tells plugin `id` `event`, when it has hooks.
    fn tell(&self, id: &str, event: Event) {
        let hooked = self.installed().iter().any(|i| i.enabled && i.manifest.id == id && i.manifest.extends("hooks"));
        if let Some(w) = hooked.then(|| self.worker(id)).flatten() {
            w.post(move |s, p| p.nori_plugin_hooks().call_on(s, &event));
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// mDNS found `service` while browsing `kind`: the plugins that browse it hear of it.
    pub fn discovered(&self, kind: String, service: Service) {
        for id in self.lan.browsers(&kind) {
            let s = types::Service { name: service.name.clone(), host: service.host.clone(), port: service.port, txt: service.txt.iter().map(|t| (t.key.clone(), t.value.clone())).collect() };
            self.tell(&id, Event::Discovered(s));
        }
    }

    /// The cast running, for the output picker and the platform's session.
    pub fn cast(&self) -> Option<Cast> {
        self.lan.cast()
    }

    /// A key or control for the cast running (the volume keys, another output picked).
    pub fn cast_button(&self, button: CastButton) {
        if let Some(c) = self.lan.cast() {
            self.tell(&c.plugin, Event::MediaButton(button.name().to_string()));
        }
    }
}
