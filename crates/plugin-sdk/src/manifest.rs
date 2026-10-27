//! What a plugin is, may reach and shows, written in Rust and kept in its wasm: `plugin!` encodes it
//! at compile time into the `nori:plugin` custom section, which nori reads before running any of it.
//! What it calls (HTTP, settings) nori reads from the wasm's imports; this says only what code can't:
//! which hosts, which keys, which rows.

/// Where a sound stage sits in nori's chain.
#[derive(Clone, Copy)]
pub enum Slot {
    /// Before transitions: one song at a time.
    Song,
    /// After the equalizer and effects.
    Master,
}

#[derive(Clone, Copy)]
pub enum RowKind {
    Switch,
    Choice,
    Level,
    /// One line, masked and kept out of logs.
    Secret,
    Text,
    Action,
    Status,
    Section,
    Note,
}

/// One row of its page; `id` is its key in the plugin's strings.
#[derive(Clone, Copy)]
pub struct Row {
    pub id: &'static str,
    pub kind: RowKind,
    /// A choice's options, keys in its strings.
    pub options: &'static [&'static str],
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    /// A switch's 0 or 1, a choice's index, a level's value.
    pub default: Option<f64>,
}

impl Row {
    pub const fn new(id: &'static str, kind: RowKind) -> Row {
        Row { id, kind, options: &[], min: None, max: None, step: None, default: None }
    }

    pub const fn level(id: &'static str, min: f64, max: f64, step: f64, default: f64) -> Row {
        Row { id, kind: RowKind::Level, options: &[], min: Some(min), max: Some(max), step: Some(step), default: Some(default) }
    }

    pub const fn switch(id: &'static str, default: bool) -> Row {
        Row { default: Some(default as u8 as f64), ..Row::new(id, RowKind::Switch) }
    }

    pub const fn choice(id: &'static str, options: &'static [&'static str], default: u32) -> Row {
        Row { options, default: Some(default as f64), ..Row::new(id, RowKind::Choice) }
    }
}

pub struct Manifest {
    /// Reverse DNS ("fm.nori.orbit").
    pub id: &'static str,
    /// A sound stage's slot, when it has one.
    pub stage: Option<Slot>,
    /// Hosts it may send HTTP to; a leading `*.` takes in their subdomains.
    pub net: &'static [&'static str],
    /// Hosts whose cookies the sign-in page hands back.
    pub signin: &'static [&'static str],
    /// nori's values it overrides ("playback.speed").
    pub overrides: &'static [&'static str],
    pub rows: &'static [Row],
}

impl Manifest {
    pub const fn new(id: &'static str) -> Manifest {
        Manifest { id, stage: None, net: &[], signin: &[], overrides: &[], rows: &[] }
    }
}

/// The plugin API this SDK speaks.
pub const API: u32 = 1;
/// The section's own format.
const FORMAT: u8 = 1;

/// Writes bytes at `at`, or only counts them when there is no room (the first pass).
struct Out<const N: usize> {
    bytes: [u8; N],
    at: usize,
}

impl<const N: usize> Out<N> {
    const fn byte(mut self, b: u8) -> Self {
        if self.at < N {
            self.bytes[self.at] = b;
        }
        self.at += 1;
        self
    }

    const fn all(mut self, b: &[u8]) -> Self {
        let mut k = 0;
        while k < b.len() {
            self = self.byte(b[k]);
            k += 1;
        }
        self
    }

    const fn u16(self, v: usize) -> Self {
        assert!(v <= u16::MAX as usize, "too long for the manifest");
        self.all(&(v as u16).to_le_bytes())
    }

    const fn u32(self, v: u32) -> Self {
        self.all(&v.to_le_bytes())
    }

    const fn str(self, s: &str) -> Self {
        self.u16(s.len()).all(s.as_bytes())
    }

    const fn strs(mut self, list: &[&str]) -> Self {
        self = self.u16(list.len());
        let mut k = 0;
        while k < list.len() {
            self = self.str(list[k]);
            k += 1;
        }
        self
    }

    /// NaN for none.
    const fn number(self, v: Option<f64>) -> Self {
        self.all(&match v {
            Some(v) => v,
            None => f64::NAN,
        }
        .to_bits()
        .to_le_bytes())
    }
}

/// Writes `m` with its version and extension points, or with `N` = 0 only counts the bytes.
const fn write<const N: usize>(m: &Manifest, version: &str, points: u32) -> Out<N> {
    let mut o = Out { bytes: [0; N], at: 0 };
    o = o.byte(FORMAT).str(m.id).str(version).u32(API).u32(points);
    o = o.byte(match m.stage {
        None => 0,
        Some(Slot::Song) => 1,
        Some(Slot::Master) => 2,
    });
    o = o.strs(m.net).strs(m.signin).strs(m.overrides).u16(m.rows.len());
    let mut k = 0;
    while k < m.rows.len() {
        let r = &m.rows[k];
        o = o.str(r.id).byte(r.kind as u8).strs(r.options).number(r.min).number(r.max).number(r.step).number(r.default);
        k += 1;
    }
    o
}

/// The section's bytes (`N` from [`encoded_len`]).
pub const fn encode<const N: usize>(m: &Manifest, version: &str, points: u32) -> [u8; N] {
    write::<N>(m, version, points).bytes
}

pub const fn encoded_len(m: &Manifest, version: &str) -> usize {
    write::<0>(m, version, 0).at
}
