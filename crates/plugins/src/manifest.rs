//! A plugin's manifest, as the SDK's `plugin!` keeps it in the wasm's `nori:plugin` custom section:
//! what it is, extends, may reach, overrides and shows. Read before any of the plugin runs.

use std::collections::BTreeMap;

/// The plugin API major version this host speaks; it also loads the one before.
pub const API: u32 = 1;

/// The extension points in the order of `wit/`'s `plugin` world exports: a manifest's bits.
pub const POINTS: [&str; 11] = ["lifecycle", "hooks", "provider", "sound-stage", "preparation", "automix", "deciders", "slots", "device-control", "lyrics", "scrobbler"];

#[derive(Debug, Clone, PartialEq)]
pub struct Manifest {
    /// Reverse DNS ("fm.nori.orbit").
    pub id: String,
    pub version: String,
    /// The `nori:plugin` major version it was built for.
    pub api: u32,
    /// Its extension points: only these of its exports are called.
    pub points: Vec<&'static str>,
    pub stage: Option<Slot>,
    /// Hosts it may send HTTP to; a leading `*.` takes in their subdomains.
    pub net: Vec<String>,
    /// Hosts whose cookies the sign-in page hands back.
    pub signin: Vec<String>,
    /// nori's values it overrides, by key ("playback.speed").
    pub overrides: Vec<String>,
    pub rows: Vec<RowSpec>,
}

/// Where a sound stage sits in the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Song,
    Master,
}

/// One row of its page, drawn by each client with its own components.
#[derive(Debug, Clone, PartialEq)]
pub struct RowSpec {
    /// Its key in the plugin's strings, and its value's in its store.
    pub id: String,
    pub kind: RowKind,
    /// A choice's options: keys in its strings.
    pub options: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// A switch's 0 or 1, a choice's index, a level's value.
    pub default: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Switch,
    Choice,
    Level,
    /// One line; masked and kept out of logs.
    Secret,
    Text,
    Action,
    Status,
    Section,
    Note,
}

impl RowKind {
    const ALL: [RowKind; 9] = [RowKind::Switch, RowKind::Choice, RowKind::Level, RowKind::Secret, RowKind::Text, RowKind::Action, RowKind::Status, RowKind::Section, RowKind::Note];

    /// Whether its value is a number a sound stage reads.
    pub fn numeric(self) -> bool {
        matches!(self, RowKind::Switch | RowKind::Choice | RowKind::Level)
    }
}

/// Why a plugin can't be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Its package, wasm or manifest can't be read.
    Unreadable(String),
    /// Built for another plugin API.
    Api(u32),
    /// It needs host interfaces this nori lacks (their WIT names).
    Needs(Vec<String>),
    /// Its id isn't reverse DNS.
    Id,
    /// It would override a value nori doesn't have.
    UnknownKey(String),
}

/// Reads the section's fields in order.
struct In<'a>(&'a [u8]);

impl<'a> In<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (a, b) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = b;
        Some(a)
    }
    fn byte(&mut self) -> Option<u8> {
        Some(self.take(1)?[0])
    }
    fn u16(&mut self) -> Option<usize> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?) as usize)
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u16()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn strs(&mut self) -> Option<Vec<String>> {
        (0..self.u16()?).map(|_| self.str()).collect()
    }
    fn number(&mut self) -> Option<Option<f64>> {
        let v = f64::from_bits(u64::from_le_bytes(self.take(8)?.try_into().ok()?));
        Some((!v.is_nan()).then_some(v))
    }
}

impl Manifest {
    /// The manifest in `wasm`'s `nori:plugin` section.
    pub fn of_wasm(wasm: &[u8]) -> Result<Manifest, Refusal> {
        let section = custom_section(wasm, "nori:plugin").ok_or_else(|| Refusal::Unreadable("no nori:plugin section (built without the SDK's plugin!)".into()))?;
        let m = Manifest::decode(&section).ok_or_else(|| Refusal::Unreadable("its manifest section can't be read".into()))?;
        if m.api != API && m.api + 1 != API {
            return Err(Refusal::Api(m.api));
        }
        let parts: Vec<&str> = m.id.split('.').collect();
        if parts.len() < 2 || parts.iter().any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')) {
            return Err(Refusal::Id);
        }
        if let Some(k) = m.overrides.iter().find(|k| !nori_settings::overrides::overridable(k)) {
            return Err(Refusal::UnknownKey(k.clone()));
        }
        Ok(m)
    }

    fn decode(bytes: &[u8]) -> Option<Manifest> {
        let mut i = In(bytes);
        (i.byte()? == 1).then_some(())?;
        let (id, version, api, bits) = (i.str()?, i.str()?, i.u32()?, i.u32()?);
        let stage = match i.byte()? {
            0 => None,
            1 => Some(Slot::Song),
            2 => Some(Slot::Master),
            _ => return None,
        };
        let (net, signin, overrides) = (i.strs()?, i.strs()?, i.strs()?);
        let rows = (0..i.u16()?)
            .map(|_| {
                Some(RowSpec { id: i.str()?, kind: *RowKind::ALL.get(i.byte()? as usize)?, options: i.strs()?, min: i.number()?, max: i.number()?, step: i.number()?, default: i.number()? })
            })
            .collect::<Option<Vec<_>>>()?;
        let points = POINTS.iter().enumerate().filter(|(k, _)| bits & (1 << k) != 0).map(|(_, p)| *p).collect();
        Some(Manifest { id, version, api, points, stage, net, signin, overrides, rows })
    }

    pub fn extends(&self, point: &str) -> bool {
        self.points.contains(&point)
    }

    /// Its numeric rows' values, in order, from what is stored (or their defaults).
    pub fn row_numbers(&self, stored: &BTreeMap<String, f64>) -> Vec<f64> {
        self.rows.iter().filter(|r| r.kind.numeric()).map(|r| stored.get(&r.id).copied().or(r.default).unwrap_or(0.0)).collect()
    }

    /// Whether HTTP to `host` is allowed.
    pub fn reaches(&self, host: &str) -> bool {
        let host = host.to_ascii_lowercase();
        self.net.iter().any(|d| match d.strip_prefix("*.") {
            Some(base) => host == base || host.ends_with(&format!(".{base}")),
            None => host == *d,
        })
    }
}

/// The first custom section `name` in `wasm`, a component's nested modules included.
fn custom_section(wasm: &[u8], name: &str) -> Option<Vec<u8>> {
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let Ok(wasmparser::Payload::CustomSection(c)) = payload {
            if c.name() == name {
                return Some(c.data().to_vec());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nori_plugin_sdk::manifest as sdk;

    /// A manifest as the SDK writes it, without a wasm around it.
    fn written(m: &sdk::Manifest, points: u32) -> Manifest {
        let bytes: [u8; 1024] = sdk::encode::<1024>(m, "1.2.3", points);
        Manifest::decode(&bytes[..sdk::encoded_len(m, "1.2.3")]).unwrap()
    }

    #[test]
    fn what_the_sdk_writes_reads_back() {
        const ROWS: [sdk::Row; 2] = [sdk::Row::level("speed", 0.02, 1.0, 0.01, 0.12), sdk::Row::choice("mode", &["a", "b"], 1)];
        let m = sdk::Manifest { stage: Some(sdk::Slot::Master), net: &["music.youtube.com"], overrides: &["playback.speed"], rows: &ROWS, ..sdk::Manifest::new("fm.nori.orbit") };
        let read = written(&m, 1 << 3);
        assert_eq!((read.id.as_str(), read.version.as_str(), read.api, read.stage), ("fm.nori.orbit", "1.2.3", API, Some(Slot::Master)));
        assert_eq!(read.points, vec!["sound-stage"]);
        assert_eq!((&read.net, &read.overrides), (&vec!["music.youtube.com".to_string()], &vec!["playback.speed".to_string()]));
        assert_eq!(read.rows[0], RowSpec { id: "speed".into(), kind: RowKind::Level, options: Vec::new(), min: Some(0.02), max: Some(1.0), step: Some(0.01), default: Some(0.12) });
        assert_eq!((read.rows[1].kind, read.rows[1].options.len(), read.rows[1].default, read.rows[1].min), (RowKind::Choice, 2, Some(1.0), None));
        assert_eq!(read.row_numbers(&BTreeMap::from([("speed".to_string(), 0.5)])), vec![0.5, 1.0]);
    }

    #[test]
    fn the_points_are_wits_in_its_order() {
        let mut resolve = wit_parser::Resolve::default();
        let (pkg, _) = resolve.push_dir(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../wit")).unwrap();
        let world = resolve.select_world(&[pkg], Some("plugin")).unwrap();
        let names: Vec<String> = resolve.worlds[world]
            .exports
            .values()
            .filter_map(|i| match i {
                wit_parser::WorldItem::Interface { id, .. } => resolve.interfaces[*id].name.clone(),
                _ => None,
            })
            .collect();
        assert_eq!(names, POINTS.to_vec());
    }

    #[test]
    fn http_reaches_only_its_hosts() {
        let m = written(&sdk::Manifest { net: &["music.youtube.com", "*.googlevideo.com"], ..sdk::Manifest::new("fm.nori.yt") }, 0);
        for (host, ok) in [("music.youtube.com", true), ("MUSIC.youtube.com", true), ("youtube.com", false), ("r3.googlevideo.com", true), ("googlevideo.com", true), ("evilgooglevideo.com", false)] {
            assert_eq!(m.reaches(host), ok, "{host}");
        }
    }
}
