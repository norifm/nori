//! A plugin's `nori:plugin` custom section, as the SDK's `plugin!` writes it: the one reader of those
//! bytes, in plain fields. The host maps them to its own kinds; the web client reads them as they are.

/// The plugin API major version this host speaks. A component's types must match exactly, so a plugin
/// built for another can't run: any change to a type in `wit/` raises it.
pub const API: u32 = 2;

/// The extension points in the order of `wit/`'s `plugin` world exports: a section's bits, in turn.
pub const POINTS: [&str; 11] = ["lifecycle", "hooks", "provider", "sound-stage", "preparation", "automix", "deciders", "slots", "device-control", "lyrics", "scrobbler"];

/// A row's kinds, in the order the SDK writes them.
pub const ROW_KINDS: [&str; 10] = ["Switch", "Choice", "Level", "Secret", "Text", "Action", "Status", "Section", "Note", "Link"];

/// A row as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: String,
    /// Into [`ROW_KINDS`].
    pub kind: u8,
    pub options: Vec<String>,
    pub min: Option<f64>,
    pub max: Option<f64>,
    pub step: Option<f64>,
    pub default: Option<f64>,
    pub hint: Option<String>,
    pub unit: Option<String>,
    pub decimals: u8,
    pub url: Option<String>,
}

/// A menu action as written: its id, glyph and the kinds it shows on.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: String,
    pub glyph: u8,
    pub kinds: Vec<u8>,
}

/// A device filter as written.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    pub name: Option<String>,
    pub service: Option<String>,
    pub vendor: u32,
}

/// A large file the manifest names.
#[derive(Debug, Clone, PartialEq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}

/// A plugin's permissions by the byte its section keeps.
pub const PERMISSIONS: [&str; 6] = ["PlaybackRead", "PlaybackControl", "LibraryRead", "LibraryWrite", "Subsonic", "Schedule"];

/// The part a plugin built before it doesn't have.
#[derive(Debug, Clone, PartialEq)]
pub struct Later {
    pub permissions: Vec<u8>,
    pub hooks: Vec<u8>,
    pub wake: Vec<u8>,
    pub commands: Vec<String>,
    pub cache_mb: u32,
    pub assets: Vec<Asset>,
}

/// What a section says.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub id: String,
    pub version: String,
    pub api: u32,
    /// Its extension points, one bit each in [`POINTS`]' order.
    pub bits: u32,
    /// 0 no stage, 1 the song slot, 2 the master's.
    pub stage: u8,
    pub net: Vec<String>,
    pub signin: Vec<String>,
    pub overrides: Vec<String>,
    pub rows: Vec<Row>,
    pub menu: Vec<Action>,
    pub places: Vec<u8>,
    pub devices: Vec<Device>,
    pub later: Option<Later>,
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
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn str(&mut self) -> Option<String> {
        let n = self.u16()? as usize;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
    fn strs(&mut self) -> Option<Vec<String>> {
        (0..self.u16()?).map(|_| self.str()).collect()
    }
    /// None for an empty string.
    fn opt_str(&mut self) -> Option<Option<String>> {
        self.str().map(|s| (!s.is_empty()).then_some(s))
    }
    fn number(&mut self) -> Option<Option<f64>> {
        let v = f64::from_bits(u64::from_le_bytes(self.take(8)?.try_into().ok()?));
        Some((!v.is_nan()).then_some(v))
    }
    fn bytes(&mut self) -> Option<Vec<u8>> {
        (0..self.u16()?).map(|_| self.byte()).collect()
    }
}

impl Section {
    /// The section in `bytes`; None when they aren't one.
    pub fn read(bytes: &[u8]) -> Option<Section> {
        let mut i = In(bytes);
        (i.byte()? == 2).then_some(())?;
        let (id, version, api, bits) = (i.str()?, i.str()?, i.u32()?, i.u32()?);
        let stage = i.byte()?;
        (stage <= 2).then_some(())?;
        let (net, signin, overrides) = (i.strs()?, i.strs()?, i.strs()?);
        let rows = (0..i.u16()?)
            .map(|_| {
                Some(Row {
                    id: i.str()?,
                    kind: i.byte()?,
                    options: i.strs()?,
                    min: i.number()?,
                    max: i.number()?,
                    step: i.number()?,
                    default: i.number()?,
                    hint: i.opt_str()?,
                    unit: i.opt_str()?,
                    decimals: i.byte()?,
                    url: i.opt_str()?,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        let menu = (0..i.u16()?).map(|_| Some(Action { id: i.str()?, glyph: i.byte()?, kinds: i.bytes()? })).collect::<Option<Vec<_>>>()?;
        let places = i.bytes()?;
        // Absent from a manifest written before it.
        let devices = if i.0.is_empty() {
            Vec::new()
        } else {
            (0..i.u16()?).map(|_| Some(Device { name: i.opt_str()?, service: i.opt_str()?, vendor: i.u32()? })).collect::<Option<Vec<_>>>()?
        };
        // A plugin built before these fields ends here.
        let later = if i.0.is_empty() {
            None
        } else {
            Some(Later {
                permissions: i.bytes()?,
                hooks: i.bytes()?,
                wake: i.bytes()?,
                commands: i.strs()?,
                cache_mb: i.u32()?,
                assets: (0..i.u16()?).map(|_| Some(Asset { name: i.str()?, url: i.str()?, size: u64::from_le_bytes(i.take(8)?.try_into().ok()?), sha256: i.str()? })).collect::<Option<Vec<_>>>()?,
            })
        };
        Some(Section { id, version, api, bits, stage, net, signin, overrides, rows, menu, places, devices, later })
    }

    /// Its extension points, by name.
    pub fn points(&self) -> Vec<&'static str> {
        POINTS.iter().enumerate().filter(|(k, _)| self.bits & (1 << k) != 0).map(|(_, p)| *p).collect()
    }
}

/// The first custom section `name` in `wasm`, a component's nested modules included.
pub fn custom_section(wasm: &[u8], name: &str) -> Option<Vec<u8>> {
    for payload in wasmparser::Parser::new(0).parse_all(wasm) {
        if let Ok(wasmparser::Payload::CustomSection(c)) = payload {
            if c.name() == name {
                return Some(c.data().to_vec());
            }
        }
    }
    None
}

/// A strings file: one `key = "text"` per line (TOML's basic strings and comments); any other line is
/// skipped.
pub fn strings(text: &str) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    for line in text.lines() {
        let Some((key, rest)) = line.split_once('=') else { continue };
        let (key, rest) = (key.trim(), rest.trim());
        let Some(body) = rest.strip_prefix('"') else { continue };
        let mut value = String::new();
        let mut chars = body.chars();
        let closed = loop {
            match chars.next() {
                None => break false,
                Some('"') => break true,
                Some('\\') => match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('u') => {
                        let hex: String = chars.by_ref().take(4).collect();
                        value.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
                    }
                    Some(c) => value.push(c),
                    None => break false,
                },
                Some(c) => value.push(c),
            }
        };
        if closed && !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            out.insert(key.to_string(), value);
        }
    }
    out
}
