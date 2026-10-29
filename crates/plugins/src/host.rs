//! The host interfaces a plugin calls, over what the rest of nori gives the plugin host ([`Side`]).

use std::collections::HashMap;

use std::sync::Arc;

use nori_net::transport::{Exchange, FailureKind, HttpMethod, Transport, TransportError};

use crate::runtime::nori::plugin::{http, log, notices, rows, settings, store, streams};
use crate::runtime::{types, Host};

/// A value of a plugin's row or one of nori's settings.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Value {
    Switch(bool),
    Choice(u32),
    Flags(u64),
    Level(f64),
    Text(String),
}

impl From<types::Value> for Value {
    fn from(v: types::Value) -> Value {
        match v {
            types::Value::Switch(b) => Value::Switch(b),
            types::Value::Choice(c) => Value::Choice(c),
            types::Value::Flags(f) => Value::Flags(f),
            types::Value::Level(l) => Value::Level(l),
            types::Value::Text(t) => Value::Text(t),
        }
    }
}

impl From<Value> for types::Value {
    fn from(v: Value) -> types::Value {
        match v {
            Value::Switch(b) => types::Value::Switch(b),
            Value::Choice(c) => types::Value::Choice(c),
            Value::Flags(f) => types::Value::Flags(f),
            Value::Level(l) => types::Value::Level(l),
            Value::Text(t) => types::Value::Text(t),
        }
    }
}

impl Value {
    /// As kept in a plugin's store: a tag, then the value.
    pub fn encode(&self) -> Vec<u8> {
        match self {
            Value::Switch(b) => vec![0, *b as u8],
            Value::Choice(c) => [&[1][..], &c.to_le_bytes()].concat(),
            Value::Flags(f) => [&[2][..], &f.to_le_bytes()].concat(),
            Value::Level(l) => [&[3][..], &l.to_le_bytes()].concat(),
            Value::Text(t) => [&[4][..], t.as_bytes()].concat(),
        }
    }

    pub fn decode(b: &[u8]) -> Option<Value> {
        let (&tag, rest) = b.split_first()?;
        Some(match tag {
            0 => Value::Switch(*rest.first()? != 0),
            1 => Value::Choice(u32::from_le_bytes(rest.try_into().ok()?)),
            2 => Value::Flags(u64::from_le_bytes(rest.try_into().ok()?)),
            3 => Value::Level(f64::from_le_bytes(rest.try_into().ok()?)),
            4 => Value::Text(String::from_utf8(rest.to_vec()).ok()?),
            _ => return None,
        })
    }

    /// As the settings read an override: `true`/`false`, a choice's index, a number, the text.
    pub fn text(&self) -> String {
        match self {
            Value::Switch(b) => b.to_string(),
            Value::Choice(c) => c.to_string(),
            Value::Flags(f) => f.to_string(),
            Value::Level(l) => l.to_string(),
            Value::Text(t) => t.clone(),
        }
    }

    /// As a sound stage reads a row: switches 0 or 1, choices their index.
    pub fn number(&self) -> Option<f64> {
        match self {
            Value::Switch(b) => Some(*b as u8 as f64),
            Value::Choice(c) => Some(*c as f64),
            Value::Level(l) => Some(*l),
            Value::Flags(_) | Value::Text(_) => None,
        }
    }
}

/// What the plugin host is given by the session that runs it.
pub trait Side: Send + Sync {
    /// The user's stored value of nori's setting `key`.
    fn stored(&self, key: &str) -> Option<Value>;
    /// Its value in effect, overrides applied.
    fn effective(&self, key: &str) -> Option<Value>;
    /// What plugin `id` lays over the settings changed (its overrides, rows, place or being on): they
    /// are derived again.
    fn laid(&self, id: &str);
    /// A status row's line changed (words from the plugin's strings).
    fn status(&self, id: &str, row: &str, words: &str, args: &[String]);
    /// A row is hidden or greyed (on top of its manifest's own conditions).
    fn shown(&self, id: &str, row: &str, shown: bool, enabled: bool);
    fn notice(&self, id: &str, problem: bool, words: &str, args: &[String]);
    /// The controlled devices or what they report changed ([`crate::Plugins::controlled`]).
    fn devices_changed(&self) {}
    /// Starts or stops browsing mDNS for `kind` ("_googlecast._tcp"); what is found goes to
    /// [`crate::Plugins::discovered`]. Asked only while a plugin browses it.
    fn browse(&self, _kind: &str, _on: bool) {}
    /// Holds the platform's multicast lock (Android's `MulticastLock`) while on: a plugin's socket is in
    /// a multicast group.
    fn multicast(&self, _on: bool) {}
    /// A cast started, changed (its device's state), or ended (None): the client holds what the
    /// platform needs for it while one runs.
    fn cast(&self, _cast: Option<&crate::Cast>) {}
}

/// Where a plugin's overrides are kept in its store.
pub(crate) const OVERRIDES: &str = "nori/override/";

fn error(kind: types::ErrorKind, detail: impl Into<String>) -> types::Error {
    types::Error { kind, detail: detail.into() }
}

impl types::Host for Host {}

impl log::Host for Host {
    /// nori's log has one level.
    fn write(&mut self, _level: log::Level, line: String) {
        nori_model::alog::info(&format!("plugin {}: {line}", self.id));
    }
}

impl store::Host for Host {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.store.get(&key)
    }

    fn set(&mut self, key: String, value: Vec<u8>) -> Result<(), types::Error> {
        if key.starts_with("nori/") {
            return Err(error(types::ErrorKind::Refused, "nori/ is the host's"));
        }
        self.store.set(&key, &value).then_some(()).ok_or_else(|| error(types::ErrorKind::Refused, "over the store's cap"))
    }

    fn remove(&mut self, key: String) {
        if !key.starts_with("nori/") {
            self.store.remove(&key);
        }
    }

    fn keys(&mut self, prefix: String) -> Vec<String> {
        self.store.keys(&prefix).into_iter().filter(|k| !k.starts_with("nori/")).collect()
    }
}

/// The host of an http(s) URL.
pub(crate) fn host_of(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    Some(match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next()?,
        None => host.split(':').next().unwrap_or(host),
    })
}

/// An answer to a plugin's request.
type Answered = Result<http::Response, types::Error>;

/// The most requests a plugin may have on their way at once.
const STARTED_MAX: usize = 16;

/// The requests a plugin started and has not taken the answers of: each runs on a thread of its own,
/// so those started together are on their way at once.
pub(crate) struct Started {
    next: u64,
    /// By id: its answer, or none while it is on its way.
    answers: HashMap<u64, Option<Answered>>,
    sent: std::sync::mpsc::Sender<(u64, Answered)>,
    came: std::sync::mpsc::Receiver<(u64, Answered)>,
}

impl Default for Started {
    fn default() -> Self {
        let (sent, came) = std::sync::mpsc::channel();
        Started { next: 0, answers: HashMap::new(), sent, came }
    }
}

impl Host {
    /// `request` as the transport sends it, once the manifest allows its host.
    fn exchange(&self, request: http::Request) -> Result<(Arc<dyn Transport>, Exchange), types::Error> {
        let host = host_of(&request.url).ok_or_else(|| error(types::ErrorKind::Refused, "not an http(s) URL"))?;
        let on_lan = || self.manifest.lan() && crate::lan::private_addresses(host, 0).is_some();
        if !self.manifest.reaches(host) && !on_lan() {
            return Err(error(types::ErrorKind::Refused, format!("{host} is not among its net hosts")));
        }
        let transport = self.transport.lock().expect("not poisoned").clone().ok_or_else(|| error(types::ErrorKind::Network, "no transport yet"))?;
        let headers: HashMap<String, String> = request.headers.into_iter().map(|h| (h.name, h.value)).collect();
        let method = match request.method {
            http::Method::Get => HttpMethod::Get,
            http::Method::Head => HttpMethod::Head,
            http::Method::Post => HttpMethod::Post,
            http::Method::Put => HttpMethod::Put,
            http::Method::Patch => HttpMethod::Patch,
            http::Method::Delete => HttpMethod::Delete,
        };
        let exchange = Exchange { url: request.url, headers, body: request.body, method: Some(method), timeout_ms: request.timeout_ms.unwrap_or(0), ..Default::default() };
        Ok((transport, exchange))
    }
}

/// `exchange` sent through `transport`, waited for.
fn sent(transport: &dyn Transport, exchange: Exchange) -> Answered {
    match nori_tasks::block_on(transport.send(exchange)) {
        Ok(r) => Ok(http::Response {
            status: r.status,
            headers: r.headers.into_iter().map(|h| types::Header { name: h.name, value: h.value }).collect(),
            body: r.body,
        }),
        Err(TransportError::Failed { kind: FailureKind::Timeout, .. }) => Err(error(types::ErrorKind::Timeout, "timed out")),
        Err(e) => Err(error(types::ErrorKind::Network, e.to_string())),
    }
}

impl http::Host for Host {
    fn fetch(&mut self, request: http::Request) -> Answered {
        let (transport, exchange) = self.exchange(request)?;
        sent(&*transport, exchange)
    }

    fn start(&mut self, request: http::Request) -> Result<u64, types::Error> {
        if self.started.answers.len() >= STARTED_MAX {
            return Err(error(types::ErrorKind::Refused, format!("more than {STARTED_MAX} requests on their way")));
        }
        let (transport, exchange) = self.exchange(request)?;
        let s = &mut self.started;
        let id = s.next;
        s.next += 1;
        s.answers.insert(id, None);
        let to = s.sent.clone();
        std::thread::Builder::new()
            .name(format!("plugin {} http", self.id))
            .spawn(move || {
                let _ = to.send((id, sent(&*transport, exchange)));
            })
            .map_err(|e| error(types::ErrorKind::Network, e.to_string()))?;
        Ok(id)
    }

    /// An id never started (or already taken) counts as answered: [`http::Host::take`] says so.
    fn wait(&mut self, ids: Vec<u64>) -> u64 {
        let s = &mut self.started;
        loop {
            if let Some(&id) = ids.iter().find(|id| !matches!(s.answers.get(id), Some(None))) {
                return id;
            }
            let Ok((id, answer)) = s.came.recv() else { return ids.first().copied().unwrap_or_default() };
            if let Some(slot) = s.answers.get_mut(&id) {
                *slot = Some(answer);
            }
        }
    }

    fn take(&mut self, id: u64) -> Option<Answered> {
        let s = &mut self.started;
        while let Ok((done, answer)) = s.came.try_recv() {
            if let Some(slot) = s.answers.get_mut(&done) {
                *slot = Some(answer);
            }
        }
        match s.answers.get(&id) {
            None => Some(Err(error(types::ErrorKind::Refused, format!("no request {id} on its way")))),
            Some(None) => None,
            Some(Some(_)) => s.answers.remove(&id).flatten(),
        }
    }
}

impl settings::Host for Host {
    fn stored(&mut self, key: String) -> Option<types::Value> {
        self.side.stored(&key).map(Into::into)
    }

    fn effective(&mut self, key: String) -> Option<types::Value> {
        self.side.effective(&key).map(Into::into)
    }

    fn override_(&mut self, values: Vec<(String, types::Value)>) -> Result<(), types::Error> {
        if let Some((k, _)) = values.iter().find(|(k, _)| !self.manifest.overrides.contains(k)) {
            return Err(error(types::ErrorKind::Refused, format!("{k} is not among its overrides")));
        }
        for (k, v) in values {
            self.store.set(&format!("{OVERRIDES}{k}"), &Value::from(v).encode());
        }
        self.side.laid(&self.id);
        Ok(())
    }

    fn clear(&mut self, keys: Vec<String>) {
        let keys = if keys.is_empty() { self.manifest.overrides.clone() } else { keys };
        for k in keys {
            self.store.remove(&format!("{OVERRIDES}{k}"));
        }
        self.side.laid(&self.id);
    }
}

impl rows::Host for Host {
    fn get(&mut self, row: String) -> Option<types::Value> {
        let stored = self.store.get(&format!("{}{row}", crate::ROWS)).and_then(|b| Value::decode(&b));
        stored.or_else(|| self.manifest.rows.iter().find(|r| r.id == row)?.initial()).map(Into::into)
    }

    fn set_status(&mut self, row: String, words: String, args: Vec<String>) {
        self.side.status(&self.id, &row, &words, &args);
    }

    fn set_shown(&mut self, row: String, shown: bool, enabled: bool) {
        self.side.shown(&self.id, &row, shown, enabled);
    }
}

impl notices::Host for Host {
    fn show(&mut self, tone: notices::Tone, words: String, args: Vec<String>) {
        self.side.notice(&self.id, tone == notices::Tone::Problem, &words, &args);
    }
}

impl streams::Host for Host {
    fn encrypted(&mut self, url: String, key: Vec<u8>, iv: Vec<u8>, skip: u64) {
        match (key.try_into(), iv.try_into()) {
            (Ok(key), Ok(iv)) => self.encrypted = Some(crate::Encrypted { url, key, iv, skip }),
            _ => nori_model::alog::info(&format!("plugin {}: an encrypted stream's key and counter are 16 bytes each", self.id)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_keep_through_the_store_and_hosts_read_from_urls() {
        for v in [Value::Switch(true), Value::Choice(3), Value::Flags(1 << 40), Value::Level(-1.5), Value::Text("é".into())] {
            assert_eq!(Value::decode(&v.encode()), Some(v));
        }
        assert_eq!(Value::decode(&[9]), None);
        for (url, host) in [("https://music.youtube.com/youtubei/v1/browse?k=1", Some("music.youtube.com")), ("http://u:p@10.0.0.2:4533/rest", Some("10.0.0.2")), ("http://[::1]:80/", Some("::1")), ("file:///etc", None)] {
            assert_eq!(host_of(url), host, "{url}");
        }
    }
}
