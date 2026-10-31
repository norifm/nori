//! What plugins reach on the LAN, one per plugin host and shared by its plugins' instances: private
//! addresses (`lan` among a manifest's hosts), the platform's multicast lock, mDNS browsing, the
//! plugins' door and the cast the host owns. Nothing here exists while no plugin asks for it.

use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv6Addr, SocketAddr, SocketAddrV6, ToSocketAddrs};
use std::sync::{Arc, Mutex, Weak};

use crate::cast::{Card, Cards, Media};
use crate::timers::{Active, Gate};
use crate::door::Door;
use crate::host::Side;
use crate::sink::Sink;
use crate::Worker;

/// What a manifest names among its hosts to reach private addresses.
pub(crate) const LAN: &str = "lan";

/// An address on this network: private (RFC 1918, RFC 4193), link-local, loopback, or a multicast
/// group of the local network's scope.
pub(crate) fn private(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_link_local() || v4.is_loopback() || (v4.is_multicast() && (v4.octets()[0] == 239 || v4.octets()[..3] == [224, 0, 0])),
        IpAddr::V6(v6) => {
            let first = v6.segments()[0];
            v6.is_loopback() || first & 0xfe00 == 0xfc00 || first & 0xffc0 == 0xfe80 || (v6.is_multicast() && matches!(first & 0x000f, 1 | 2 | 5)) || v6.to_ipv4_mapped().is_some_and(|v4| private(IpAddr::V4(v4)))
        }
    }
}

/// An IPv6 address with its zone, the index of the interface it is on (`fe80::1%3`), as a plugin hears
/// a link-local sender.
pub(crate) fn zoned(host: &str) -> Option<(Ipv6Addr, u32)> {
    let (ip, zone) = host.split_once('%')?;
    Some((ip.parse().ok()?, zone.parse().ok()?))
}

/// A sender's address as a plugin hears it: IPv4 as IPv4, not mapped into IPv6; a link-local one with
/// its zone, so that a reply reaches it.
pub(crate) fn host_of(from: SocketAddr) -> String {
    match from {
        SocketAddr::V6(v6) if v6.scope_id() != 0 => format!("{}%{}", v6.ip(), v6.scope_id()),
        from => from.ip().to_canonical().to_string(),
    }
}

/// `host`:`port`'s addresses, when every one of them is private; None when any is not (a name that
/// resolves outside the LAN is not the LAN's) or it has none.
pub(crate) fn private_addresses(host: &str, port: u16) -> Option<Vec<SocketAddr>> {
    let all: Vec<SocketAddr> = match zoned(host) {
        Some((ip, zone)) => vec![SocketAddr::V6(SocketAddrV6::new(ip, port, 0, zone))],
        None => (host, port).to_socket_addrs().ok()?.collect(),
    };
    (!all.is_empty() && all.iter().all(|a| private(a.ip()))).then_some(all)
}

/// This device's address on the LAN: the one a route to a LAN address leaves from (no packet is sent).
pub(crate) fn lan_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    socket.local_addr().ok().map(|a| a.ip()).filter(|ip| !ip.is_unspecified())
}

/// A cast the host owns while a plugin runs one.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Cast {
    pub plugin: String,
    pub kind: CastKind,
    /// The device's name, as the plugin said it.
    pub device: String,
    pub look: CastLook,
    pub playing: bool,
    pub position_ms: u64,
    /// 0 to 100, when the device says.
    pub volume: Option<u8>,
    /// A deep sink: the phone sends its audio in bursts, so the Wi-Fi lock is held only while one is
    /// sent ([`crate::Client::streaming`]), not for the whole cast.
    pub bursts: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum CastKind {
    /// nori's final audio goes to the device (`audio-out`): the phone streams it.
    Sink,
    /// The device fetches the songs itself (`media`): the phone may sleep.
    Target,
}

/// What a device asks of a cast: the media buttons the plugin hears (`hooks.on(media-button(..))`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum CastButton {
    VolumeUp,
    VolumeDown,
    /// The user picked another output: the plugin ends its cast.
    Stop,
}

impl CastButton {
    /// As the plugin hears it.
    pub(crate) fn name(self) -> &'static str {
        match self {
            CastButton::VolumeUp => "volume-up",
            CastButton::VolumeDown => "volume-down",
            CastButton::Stop => "cast-stop",
        }
    }
}

/// The enabled plugins' instances, by id.
pub(crate) type Workers = Mutex<BTreeMap<String, Worker>>;

/// A device a plugin can cast to, for a client's devices list.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct CastTarget {
    pub plugin: String,
    pub id: String,
    pub name: String,
    pub look: CastLook,
}

/// What a device a plugin casts to is, for its icon.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum CastLook {
    #[default]
    Speaker,
    Headphones,
    Phone,
    Tv,
}

#[derive(Default)]
struct Inner {
    /// What plugins said their devices are, by plugin and target.
    looks: BTreeMap<(String, String), CastLook>,
    /// What each plugin offers to cast to.
    targets: BTreeMap<String, Vec<CastTarget>>,
    /// Sockets that joined a multicast group, across all plugins: the lock is held while any is.
    multicast: u32,
    /// mDNS kinds being browsed, and the plugins that asked for each.
    browsing: BTreeMap<String, BTreeSet<String>>,
    /// Plugins whose `lan-serve` endpoints are open.
    serving: BTreeSet<String>,
    /// Song URLs the door proxies for plugins (`media`), by index: the plugin that asked, the URL.
    proxied: Vec<Option<(String, String)>>,
    /// The audio served at the door (`audio-out.serve`), by the plugin that serves it.
    served: Option<(String, Arc<Sink>)>,
    /// The audio pumped into a plugin's socket (`audio-out.to-socket`).
    pumped: Option<(String, Arc<Sink>)>,
    cast: Option<Cast>,
    door: Option<Door>,
}

impl Inner {
    fn door_wanted(&self) -> bool {
        !self.serving.is_empty() || self.served.is_some() || self.proxied.iter().any(Option::is_some)
    }
}

pub(crate) struct Lan {
    side: Arc<dyn Side>,
    workers: Weak<Workers>,
    /// Where the cast makes its plugin's session active, for its timers.
    gate: Arc<Gate>,
    /// The host, which a lost sink's plugin is told through.
    plugins: std::sync::OnceLock<Weak<crate::Plugins>>,
    inner: Mutex<Inner>,
    media: Mutex<Option<Weak<dyn Media>>>,
    cards: Mutex<Option<Cards>>,
    /// Itself, for the door's handler.
    me: Weak<Lan>,
}

/// Held by a UDP socket that joined a group: the multicast lock is let go when the last goes.
pub(crate) struct Multicast(Arc<Lan>);

impl Drop for Multicast {
    fn drop(&mut self) {
        let mut i = self.0.inner.lock().expect("not poisoned");
        i.multicast -= 1;
        if i.multicast == 0 {
            self.0.side.multicast(false);
        }
    }
}

impl Lan {
    pub(crate) fn new(side: Arc<dyn Side>, workers: Weak<Workers>, gate: Arc<Gate>) -> Arc<Lan> {
        Arc::new_cyclic(|me| Lan { side, workers, gate, plugins: std::sync::OnceLock::new(), inner: Mutex::default(), media: Mutex::new(None), cards: Mutex::new(None), me: me.clone() })
    }

    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().expect("not poisoned")
    }

    /// The host a lost sink's plugin is told through.
    pub(crate) fn set_plugins(&self, plugins: Weak<crate::Plugins>) {
        let _ = self.plugins.set(plugins);
    }

    pub(crate) fn set_media(&self, media: Option<Weak<dyn Media>>) {
        *self.media.lock().expect("not poisoned") = media;
    }

    pub(crate) fn media(&self) -> Option<Arc<dyn Media>> {
        self.media.lock().expect("not poisoned").as_ref()?.upgrade()
    }

    pub(crate) fn set_cards(&self, cards: Cards) {
        *self.cards.lock().expect("not poisoned") = Some(cards);
    }

    /// Where nori's music plays from now on.
    fn card(&self, card: Card) {
        match self.cards.lock().expect("not poisoned").as_ref() {
            Some(cards) => cards(card),
            None => nori_model::alog::info("plugins: no engine takes a cast's audio here"),
        }
    }

    pub(crate) fn worker(&self, id: &str) -> Option<Worker> {
        self.workers.upgrade()?.lock().expect("not poisoned").get(id).cloned()
    }

    pub(crate) fn multicast(self: &Arc<Self>) -> Multicast {
        let mut i = self.inner();
        i.multicast += 1;
        if i.multicast == 1 {
            self.side.multicast(true);
        }
        Multicast(self.clone())
    }

    pub(crate) fn browse(&self, id: &str, kind: &str) {
        let mut i = self.inner();
        let asked = i.browsing.entry(kind.to_string()).or_default();
        if asked.insert(id.to_string()) && asked.len() == 1 {
            self.side.browse(kind, true);
        }
    }

    pub(crate) fn stop_browsing(&self, id: &str, kind: &str) {
        let mut i = self.inner();
        let Some(asked) = i.browsing.get_mut(kind) else { return };
        if asked.remove(id) && asked.is_empty() {
            i.browsing.remove(kind);
            self.side.browse(kind, false);
        }
    }

    /// The plugins browsing `kind`.
    pub(crate) fn browsers(&self, kind: &str) -> Vec<String> {
        self.inner().browsing.get(kind).map(|s| s.iter().cloned().collect()).unwrap_or_default()
    }

    /// The door's base URL, once it is open.
    pub(crate) fn base(&self) -> Option<String> {
        let port = self.inner().door.as_ref()?.port();
        Some(format!("http://{}", SocketAddr::new(lan_ip()?, port)))
    }

    /// Opens the door when it is wanted and is not open; closes it when it is not wanted.
    fn keep_door(&self, i: &mut Inner) {
        if !i.door_wanted() {
            i.door = None;
        } else if i.door.is_none() {
            let me = self.me.clone();
            i.door = Door::open(Box::new(move |request, conn| {
                if let Some(lan) = me.upgrade() {
                    lan.answer(request, conn);
                }
            }))
            .map_err(|e| nori_model::alog::info(&format!("plugins: no door: {e}")))
            .ok();
        }
    }

    pub(crate) fn serve(&self, id: &str, on: bool) {
        let mut i = self.inner();
        if on {
            i.serving.insert(id.to_string());
        } else {
            i.serving.remove(id);
        }
        self.keep_door(&mut i);
    }

    /// `url` at the door, for another device on the LAN.
    pub(crate) fn proxy(&self, id: &str, url: String) -> Option<String> {
        let mut i = self.inner();
        let at = match i.proxied.iter().position(|p| p.as_ref().is_some_and(|(by, u)| by == id && *u == url)) {
            Some(at) => at,
            None => {
                i.proxied.push(Some((id.to_string(), url)));
                i.proxied.len() - 1
            }
        };
        self.keep_door(&mut i);
        drop(i);
        Some(format!("{}/m/{at}", self.base()?))
    }

    /// Plugin `id`'s audio served at the door from now on; its URL.
    pub(crate) fn serve_sink(&self, id: &str, sink: Arc<Sink>) -> Option<String> {
        let mut i = self.inner();
        let extension = sink.encoding.extension();
        i.served = Some((id.to_string(), sink.clone()));
        self.keep_door(&mut i);
        drop(i);
        let url = self.base().map(|b| format!("{b}/a/{extension}"));
        if url.is_some() {
            self.card(Card::Sink(sink));
        }
        url
    }

    pub(crate) fn pump_sink(&self, id: &str, sink: Arc<Sink>) {
        if let Some(deep) = &sink.deep {
            let (me, plugin) = (self.me.clone(), id.to_string());
            deep.controls(Box::new(move |said| {
                let Some(lan) = me.upgrade() else { return };
                match said {
                    Some(said) => lan.device_said(&plugin, said),
                    // Off the socket's own thread, which letting the sink go waits for.
                    None => {
                        let plugin = plugin.clone();
                        drop(std::thread::Builder::new().name("nori-cast-lost".into()).spawn(move || lan.sink_lost(&plugin)));
                    }
                }
            }));
            let side = self.side.clone();
            deep.on_burst(Box::new(move |on| side.streaming(on)));
        }
        self.inner().pumped = Some((id.to_string(), sink.clone()));
        self.card(Card::Sink(sink));
    }

    /// Plugin `id`'s deep sink went away: nori plays on its own output again, and the plugin hears the
    /// cast stop as if another output were picked. Nothing when the plugin stopped it first.
    fn sink_lost(&self, id: &str) {
        if !self.inner().pumped.as_ref().is_some_and(|(by, s)| by == id && s.deep.is_some()) {
            return;
        }
        self.stop_sink(id);
        self.end_cast(id);
        if let Some(plugins) = self.plugins.get().and_then(Weak::upgrade) {
            plugins.tell_one(id, crate::runtime::points::hooks::Event::MediaButton(CastButton::Stop.name().to_string()));
        }
    }

    /// A deep sink's button or knob.
    fn device_said(&self, id: &str, said: crate::deep::Device) {
        use crate::deep::{Button, Device};
        use nori_model::deciders::Press;
        match said {
            Device::Press(b) => self.side.pressed(match b {
                Button::PlayPause => Press::PlayPause,
                Button::Next => Press::Next,
                Button::Previous => Press::Previous,
            }),
            Device::Volume { gain } => {
                let mut i = self.inner();
                let Some(cast) = i.cast.as_mut().filter(|c| c.plugin == id) else { return };
                cast.volume = Some((gain.clamp(0.0, 1.0) * 100.0).round() as u8);
                let cast = cast.clone();
                drop(i);
                self.side.cast(Some(&cast));
            }
            _ => {}
        }
    }

    /// A deep sink's volume is `volume` (of 100) from now on, sent to the device; false when no deep
    /// sink plays.
    pub(crate) fn deep_volume(&self, volume: u8) -> bool {
        let mut i = self.inner();
        let Some(deep) = i.pumped.as_ref().and_then(|(_, s)| s.deep.clone()) else { return false };
        let Some(cast) = i.cast.as_mut() else { return false };
        cast.volume = Some(volume);
        let cast = cast.clone();
        drop(i);
        deep.set_volume(volume as f32 / 100.0);
        self.side.cast(Some(&cast));
        true
    }

    pub(crate) fn sink_latency(&self, id: &str, ms: u32) {
        let i = self.inner();
        for (by, sink) in i.served.iter().chain(i.pumped.iter()) {
            if by == id {
                sink.set_latency_ms(ms);
            }
        }
    }

    /// Plugin `id`'s audio stops: nori plays on its own output again.
    pub(crate) fn stop_sink(&self, id: &str) {
        let mut i = self.inner();
        let served = i.served.as_ref().is_some_and(|(by, _)| by == id).then(|| i.served.take()).flatten();
        let pumped = i.pumped.as_ref().is_some_and(|(by, _)| by == id).then(|| i.pumped.take()).flatten();
        self.keep_door(&mut i);
        drop(i);
        if served.or(pumped).is_some() {
            self.card(Card::Here);
        }
    }

    /// Target `target` of plugin `id` is a `look`.
    pub(crate) fn show_as(&self, id: &str, target: String, look: CastLook) {
        let mut i = self.inner();
        for t in i.targets.get_mut(id).into_iter().flatten().filter(|t| t.id == target) {
            t.look = look;
        }
        i.looks.insert((id.to_string(), target), look);
        drop(i);
        self.side.devices_changed();
    }

    /// What plugin `id` can cast to now.
    pub(crate) fn offer(&self, id: &str, targets: Vec<(String, String)>) {
        let plugin = id.to_string();
        let mut i = self.inner();
        let offered = targets.into_iter().map(|(id, name)| CastTarget { look: i.looks.get(&(plugin.clone(), id.clone())).copied().unwrap_or_default(), plugin: plugin.clone(), id, name }).collect();
        i.targets.insert(plugin, offered);
        drop(i);
        self.side.devices_changed();
    }

    /// The devices plugins can cast to, but the one playing.
    pub(crate) fn targets(&self) -> Vec<CastTarget> {
        let i = self.inner();
        let playing = i.cast.as_ref().map(|c| c.device.as_str());
        i.targets.values().flatten().filter(|t| Some(t.name.as_str()) != playing).cloned().collect()
    }

    pub(crate) fn start_cast(&self, id: &str, kind: CastKind, device: String) {
        let mut i = self.inner();
        let bursts = kind == CastKind::Sink && i.pumped.as_ref().is_some_and(|(by, s)| by == id && s.deep.is_some());
        let look = i.targets.get(id).into_iter().flatten().find(|t| t.name == device).map(|t| t.look).unwrap_or_default();
        let cast = Cast { plugin: id.to_string(), kind, device, look, playing: false, position_ms: 0, volume: None, bursts };
        if i.cast.as_ref().is_some_and(|c| c.plugin != id) {
            nori_model::alog::info(&format!("plugin {id}: another plugin's cast runs"));
            return;
        }
        let first = i.cast.replace(cast.clone()).is_none();
        drop(i);
        self.gate.active(id, Active::Cast, true);
        if first && kind == CastKind::Target {
            self.card(Card::Elsewhere { playing: None, position_ms: None });
        }
        self.side.cast(Some(&cast));
    }

    pub(crate) fn report_cast(&self, id: &str, playing: bool, position_ms: u64, volume: Option<u8>) {
        let mut i = self.inner();
        let Some(cast) = i.cast.as_mut().filter(|c| c.plugin == id) else { return };
        (cast.playing, cast.position_ms, cast.volume) = (playing, position_ms, volume);
        let cast = cast.clone();
        drop(i);
        if cast.kind == CastKind::Target {
            self.card(Card::Elsewhere { playing: Some(playing), position_ms: Some(position_ms) });
        }
        self.side.cast(Some(&cast));
    }

    pub(crate) fn end_cast(&self, id: &str) {
        let mut i = self.inner();
        if let Some(c) = i.cast.take_if(|c| c.plugin == id) {
            drop(i);
            self.gate.active(id, Active::Cast, false);
            if c.kind == CastKind::Target {
                self.card(Card::Back);
            }
            self.side.cast(None);
        }
    }

    pub(crate) fn cast(&self) -> Option<Cast> {
        self.inner().cast.clone()
    }

    /// Lets go of all plugin `id` holds here: it stopped, was turned off or its instance failed.
    pub(crate) fn forget(&self, id: &str) {
        self.inner().targets.remove(id);
        let kinds: Vec<String> = self.inner().browsing.iter().filter(|(_, by)| by.contains(id)).map(|(k, _)| k.clone()).collect();
        for kind in kinds {
            self.stop_browsing(id, &kind);
        }
        {
            let mut i = self.inner();
            i.serving.remove(id);
            for p in i.proxied.iter_mut().filter(|p| p.as_ref().is_some_and(|(by, _)| by == id)) {
                *p = None;
            }
            self.keep_door(&mut i);
        }
        self.stop_sink(id);
        self.end_cast(id);
    }

    /// Answers a request at the door.
    fn answer(&self, request: crate::door::Request, conn: std::net::TcpStream) {
        use crate::door::{respond, Response};
        let path = request.path.clone();
        let mut parts = path.trim_start_matches('/').splitn(3, '/');
        match (parts.next(), parts.next(), parts.next()) {
            (Some("p"), Some(id), rest) => {
                let query = request.query.as_ref().map(|q| format!("?{q}")).unwrap_or_default();
                let rest = format!("/{}{query}", rest.unwrap_or_default());
                let serving = self.inner().serving.contains(id);
                let answer = serving.then(|| self.worker(id)).flatten().and_then(|w| {
                    let asked = crate::runtime::points::hooks::ServedRequest { method: request.method, path: rest, headers: request.headers, body: request.body };
                    nori_tasks::block_on(w.call(move |s, p| p.nori_plugin_hooks().call_served(s, &asked))).flatten()
                });
                match answer {
                    Some(r) => respond(conn, &Response { status: r.status, headers: r.headers, body: r.body }),
                    None => respond(conn, &Response::status(404)),
                }
            }
            (Some("a"), Some(_), None) => match self.inner().served.as_ref().map(|(_, s)| s.clone()) {
                Some(sink) => sink.listen(conn),
                None => respond(conn, &Response::status(404)),
            },
            (Some("m"), Some(at), None) => {
                let url = at.parse::<usize>().ok().and_then(|at| self.inner().proxied.get(at).cloned().flatten()).map(|(_, u)| u);
                match url {
                    Some(url) => crate::door::forward(&url, &request, conn),
                    None => respond(conn, &Response::status(404)),
                }
            }
            _ => respond(conn, &Response::status(404)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_networks_addresses_are_private() {
        for (ip, lan) in [
            ("192.168.1.20", true),
            ("10.0.2.2", true),
            ("172.16.0.1", true),
            ("172.32.0.1", false),
            ("169.254.3.4", true),
            ("127.0.0.1", true),
            ("239.255.255.250", true),
            ("224.0.0.251", true),
            ("224.1.0.1", false),
            ("8.8.8.8", false),
            ("fd12::1", true),
            ("fe80::1", true),
            ("ff02::fb", true),
            ("ff0e::1", false),
            ("2001:db8::1", false),
            ("::ffff:192.168.0.1", true),
            ("::ffff:1.1.1.1", false),
        ] {
            assert_eq!(private(ip.parse().unwrap()), lan, "{ip}");
        }
        assert_eq!(private_addresses("127.0.0.1", 80).map(|a| a.len()), Some(1));
        assert_eq!(private_addresses("1.1.1.1", 80), None);
        assert_eq!(private_addresses("fe80::1%3", 80), Some(vec!["[fe80::1%3]:80".parse().unwrap()]));
        assert_eq!(private_addresses("2001:db8::1%3", 80), None);
    }

    #[test]
    fn a_sender_is_heard_as_a_reply_reaches_it() {
        for (from, heard) in [("192.168.1.2:9", "192.168.1.2"), ("[::ffff:192.168.1.2]:9", "192.168.1.2"), ("[fe80::1%4]:9", "fe80::1%4"), ("[fd00::2]:9", "fd00::2")] {
            assert_eq!(host_of(from.parse().unwrap()), heard, "{from}");
        }
    }
}
