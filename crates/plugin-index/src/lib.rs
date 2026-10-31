//! A plugin repo's `index.json`, and what stops a plugin running in a browser: pure, with no plugin
//! host, so the web client reads it too. [`web_blocks`] is the one rule, for a listing and an install.

pub mod section;

use section::Section;
use serde::{Deserialize, Serialize};

/// nori's own repo: nori-plugins' index (its packages are that repository's releases).
pub const OFFICIAL: &str = "https://raw.githubusercontent.com/norifm/nori-plugins/main/index.json";

/// What a repo's `index.json` holds.
#[derive(Debug, Deserialize)]
pub struct Index {
    pub plugins: Vec<Listed>,
}

/// A plugin as an index lists it.
#[derive(Debug, Deserialize)]
pub struct Listed {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub about: Option<String>,
    /// Where its code is.
    #[serde(default)]
    pub source: Option<String>,
    /// Its README, absolute or relative to the index.
    #[serde(default)]
    pub readme: Option<String>,
    /// Its icon, absolute or relative to the index.
    #[serde(default)]
    pub icon: Option<String>,
    /// The extension points it fills (`wit/`'s names).
    #[serde(default)]
    pub points: Vec<String>,
    /// The host interfaces it calls ([`imports`]); none: the index doesn't say.
    #[serde(default)]
    pub imports: Option<Vec<String>>,
    /// The hosts it reaches, `lan` among them when it does (the manifest's `net`).
    #[serde(default)]
    pub net: Vec<String>,
    pub versions: Vec<Release>,
}

#[derive(Debug, Deserialize)]
pub struct Release {
    pub version: String,
    pub api: u32,
    #[serde(default)]
    pub released: Option<String>,
    /// Absolute, or relative to the index.
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub signature: Option<String>,
}

/// An `index.json`'s text as an index.
pub fn parse(text: &[u8]) -> Result<Index, String> {
    serde_json::from_slice(text).map_err(|e| format!("not an index: {e}"))
}

/// What a browser has no way to give a plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum WebBlock {
    /// Raw TCP and UDP.
    Sockets,
    /// Devices on the local network.
    LocalNetwork,
    /// mDNS.
    Discovery,
    /// Serving other devices.
    Serving,
    /// Handing nori's audio to a device.
    Casting,
    Bluetooth,
    /// Headphone control, which is Bluetooth's.
    DeviceControl,
    /// Starting while nori is closed.
    Wake,
    /// A sign-in page handing back another site's cookies.
    SignIn,
    /// Requests to sites that don't allow a web page's, with no proxy to send them through.
    OtherSites,
    /// A kind of profile to add: the web's profiles are servers.
    Provider,
    /// Changing nori's own settings.
    Overrides,
    /// An extension point or host call the web's host doesn't have yet.
    NotYet,
}

/// The extension points the web's host calls.
const WEB_POINTS: [&str; 4] = ["lifecycle", "hooks", "sound-stage", "slots"];

/// The host calls the web's host answers, and those only a client's own stack can.
const WEB_IMPORTS: [&str; 7] = ["log", "store", "cache", "http", "notices", "rows", "timers"];

/// The name of the host interface in a component's import (`nori:plugin/sockets@2.0.0`).
fn interface(import: &str) -> Option<&str> {
    let name = import.strip_prefix("nori:plugin/")?;
    Some(name.split('@').next().unwrap_or(name))
}

/// What stops a plugin running in a browser, from what it fills, the hosts it names and the host
/// interfaces it imports (as [`imports`] names them), each reason once; none: it can. With `proxied`,
/// the server this page came from carries a plugin's requests to the sites it names.
pub fn web_blocks(points: &[String], net: &[String], imports: &[String], proxied: bool) -> Vec<WebBlock> {
    use WebBlock::*;
    let mut blocks = Vec::new();
    let mut add = |b: WebBlock| {
        if !blocks.contains(&b) {
            blocks.push(b);
        }
    };
    for point in points {
        match point.as_str() {
            "device-control" => add(DeviceControl),
            "provider" => add(Provider),
            p if !WEB_POINTS.contains(&p) => add(NotYet),
            _ => {}
        }
    }
    if net.iter().any(|h| h == "lan") {
        add(LocalNetwork);
    }
    for import in imports {
        match import.as_str() {
            "sockets" => add(Sockets),
            "discovery" => add(Discovery),
            "lan-serve" => add(Serving),
            "audio-out" | "cast" | "media" => add(Casting),
            "bluetooth" => add(Bluetooth),
            "wake" => add(Wake),
            "sign-in" => add(SignIn),
            "settings" => add(Overrides),
            "http" if !proxied && net.iter().any(|h| h != "lan") => add(OtherSites),
            "types" => {}
            i if !WEB_IMPORTS.contains(&i) => add(NotYet),
            _ => {}
        }
    }
    blocks
}

/// Whether a listed plugin can run in a browser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "state", content = "blocks")]
pub enum WebFit {
    Supported,
    /// What stops it, every reason.
    Blocked(Vec<WebBlock>),
    /// Nothing listed stops it, but its index doesn't say what it imports.
    Unknown,
}

/// [`web_blocks`] of what `listed` says of itself.
pub fn web_fit(listed: &Listed, proxied: bool) -> WebFit {
    let imports = listed.imports.as_deref();
    let blocks = web_blocks(&listed.points, &listed.net, imports.unwrap_or_default(), proxied);
    match (blocks.is_empty(), imports) {
        (false, _) => WebFit::Blocked(blocks),
        (true, Some(_)) => WebFit::Supported,
        (true, None) => WebFit::Unknown,
    }
}

/// A row of a plugin as a browser draws it, in the plugin's words.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebRow {
    pub id: String,
    /// One of [`section::ROW_KINDS`].
    pub kind: &'static str,
    /// A number to a sound stage: a switch's 0 or 1, a choice's index, a level.
    pub numeric: bool,
    pub label: String,
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

/// A package as the web client installs it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WebPlugin {
    pub id: String,
    pub version: String,
    pub name: String,
    pub about: Option<String>,
    pub points: Vec<&'static str>,
    pub stage: bool,
    pub net: Vec<String>,
    pub rows: Vec<WebRow>,
    pub blocks: Vec<WebBlock>,
}

/// What `section` of a plugin importing `imports` is in a browser, in the words of its strings file.
pub fn web_plugin(section: &Section, imports: &[String], strings: &str, proxied: bool) -> WebPlugin {
    let words = section::strings(strings);
    let said = |k: &str| words.get(k).cloned();
    let points = section.points();
    let named: Vec<String> = points.iter().map(|p| p.to_string()).collect();
    WebPlugin {
        id: section.id.clone(),
        version: section.version.clone(),
        name: said("name").unwrap_or_else(|| section.id.clone()),
        about: said("about"),
        stage: section.stage != 0,
        net: section.net.clone(),
        blocks: web_blocks(&named, &section.net, imports, proxied),
        points,
        rows: section
            .rows
            .iter()
            .map(|r| {
                let kind = section::ROW_KINDS.get(r.kind as usize).copied().unwrap_or("Note");
                WebRow {
                    id: r.id.clone(),
                    kind,
                    numeric: matches!(kind, "Switch" | "Choice" | "Level"),
                    label: said(&r.id).unwrap_or_else(|| r.id.clone()),
                    options: r.options.iter().map(|o| said(o).unwrap_or_else(|| o.clone())).collect(),
                    min: r.min,
                    max: r.max,
                    step: r.step,
                    default: r.default,
                    hint: r.hint.as_deref().and_then(&said),
                    unit: r.unit.as_deref().and_then(&said),
                    decimals: r.decimals,
                    url: r.url.clone(),
                }
            })
            .collect(),
    }
}

/// The package bytes' `plugin.wasm` as [`web_plugin`] reads it; an error when it isn't a plugin of this API.
pub fn read_web_plugin(wasm: &[u8], strings: &str, proxied: bool) -> Result<WebPlugin, String> {
    let bytes = section::custom_section(wasm, "nori:plugin").ok_or("no manifest")?;
    let section = Section::read(&bytes).ok_or("unreadable manifest")?;
    if section.api != section::API {
        return Err(format!("built for plugin API {}", section.api));
    }
    Ok(web_plugin(&section, &imports(wasm)?, strings, proxied))
}

/// The host interfaces a plugin's component imports, by name, for [`web_blocks`] and an index.
pub fn imports(wasm: &[u8]) -> Result<Vec<String>, String> {
    use wasmparser::{Parser, Payload};
    let mut out = Vec::new();
    for payload in Parser::new(0).parse_all(wasm) {
        if let Payload::ComponentImportSection(section) = payload.map_err(|e| e.to_string())? {
            for import in section {
                let import = import.map_err(|e| e.to_string())?;
                if let Some(name) = interface(import.name.name) {
                    if !out.iter().any(|n| n == name) {
                        out.push(name.to_string());
                    }
                }
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use WebBlock::*;

    fn all(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn what_a_plugin_needs_says_what_stops_it_in_a_browser() {
        for (points, net, imports, proxied, want) in [
            (vec!["sound-stage"], vec![], vec!["log", "store"], false, vec![]),
            (vec!["lifecycle"], vec!["lan"], vec!["sockets", "discovery", "cast", "audio-out"], false, vec![LocalNetwork, Sockets, Discovery, Casting]),
            (vec!["device-control"], vec![], vec!["bluetooth"], false, vec![DeviceControl, Bluetooth]),
            (vec!["provider"], vec!["music.youtube.com"], vec!["http", "sign-in", "wake"], true, vec![Provider, SignIn, Wake]),
            (vec!["lifecycle"], vec!["lan"], vec!["http"], false, vec![LocalNetwork]),
            (vec!["lifecycle"], vec![], vec!["cast", "audio-out", "media"], false, vec![Casting]),
            (vec!["slots"], vec!["amp-api.music.apple.com"], vec!["http", "log", "store"], false, vec![OtherSites]),
            (vec!["slots"], vec!["amp-api.music.apple.com"], vec!["http", "log", "store"], true, vec![]),
            (vec!["lifecycle", "preparation"], vec![], vec!["log"], false, vec![NotYet]),
            (vec!["lifecycle"], vec![], vec!["settings", "rows"], false, vec![Overrides]),
            (vec!["hooks"], vec![], vec!["session", "subsonic", "assets"], false, vec![NotYet]),
        ] {
            assert_eq!(web_blocks(&all(&points), &all(&net), &all(&imports), proxied), want, "{points:?} {net:?} {imports:?} {proxied}");
        }
    }

    #[test]
    fn a_listing_is_called_supported_only_when_it_says_what_it_imports() {
        let listed = |points: &[&str], imports: Option<&[&str]>| Listed { id: "a".into(), name: "A".into(), about: None, source: None, readme: None, icon: None, points: all(points), imports: imports.map(all), net: Vec::new(), versions: Vec::new() };
        assert_eq!(web_fit(&listed(&["lifecycle"], None), false), WebFit::Unknown);
        assert_eq!(web_fit(&listed(&["lifecycle"], Some(&["log"])), false), WebFit::Supported);
        assert_eq!(web_fit(&listed(&["device-control"], None), false), WebFit::Blocked(vec![DeviceControl]), "what is known stops it already");
        assert_eq!(web_fit(&listed(&["lifecycle"], Some(&["sockets", "bluetooth"])), false), WebFit::Blocked(vec![Sockets, Bluetooth]));
    }

    #[test]
    fn an_index_reads_with_or_without_what_a_plugin_imports() {
        let text = br#"{"plugins":[{"id":"a","name":"A","versions":[{"version":"1","api":2,"url":"a","sha256":"0"}]},{"id":"b","name":"B","imports":["http"],"net":["x.example"],"versions":[]}]}"#;
        let index = parse(text).unwrap();
        assert_eq!(index.plugins[0].imports, None);
        assert_eq!(index.plugins[1].imports.as_deref(), Some(&["http".to_string()][..]));
        assert!(parse(b"[]").is_err());
    }

    #[test]
    fn a_section_reads_as_the_sdk_wrote_it() {
        fn string(o: &mut Vec<u8>, s: &str) {
            o.extend((s.len() as u16).to_le_bytes());
            o.extend(s.as_bytes());
        }
        let mut b = vec![2];
        string(&mut b, "fm.nori.a");
        string(&mut b, "1.2.3");
        b.extend(2u32.to_le_bytes());
        b.extend(0b1001u32.to_le_bytes());
        b.push(2);
        b.extend(1u16.to_le_bytes());
        string(&mut b, "lan");
        b.extend([0, 0, 0, 0]);
        b.extend(1u16.to_le_bytes());
        string(&mut b, "gain");
        b.push(2);
        b.extend(0u16.to_le_bytes());
        for v in [Some(-1.0), Some(2.0), None, Some(0.5)] {
            b.extend(v.unwrap_or(f64::NAN).to_le_bytes());
        }
        string(&mut b, "");
        string(&mut b, "db");
        b.push(1);
        string(&mut b, "");
        b.extend([0, 0, 0, 0]);
        let s = section::Section::read(&b).unwrap();
        assert_eq!((s.id.as_str(), s.api, s.stage, s.points()), ("fm.nori.a", 2, 2, vec!["lifecycle", "sound-stage"]));
        assert_eq!(s.net, ["lan"]);
        let r = &s.rows[0];
        assert_eq!((r.id.as_str(), section::ROW_KINDS[r.kind as usize], r.min, r.step, r.default), ("gain", "Level", Some(-1.0), None, Some(0.5)));
        assert_eq!((r.hint.clone(), r.unit.as_deref()), (None, Some("db")));
        assert_eq!(s.later, None, "written before those fields");
        assert_eq!(section::Section::read(&b[..9]), None);
    }

    #[test]
    fn strings_read_one_per_line() {
        let w = section::strings("# Orbit\nname = \"Orbit\"\nabout = \"Circles \\\"you\\\"\\nround\"\nunit-s = \"\\u00e9\"\n[table]\nbroken = \"no end\nwhat\n");
        assert_eq!(w.get("name").map(String::as_str), Some("Orbit"));
        assert_eq!(w.get("about").map(String::as_str), Some("Circles \"you\"\nround"));
        assert_eq!(w.get("unit-s").map(String::as_str), Some("é"));
        assert_eq!(w.len(), 3);
    }

    #[test]
    fn a_plugin_reads_for_a_browser_in_its_words() {
        let section = Section {
            id: "fm.nori.orbit".into(),
            version: "0.2.0".into(),
            api: section::API,
            bits: 1 << 3,
            stage: 2,
            net: vec![],
            signin: vec![],
            overrides: vec![],
            rows: vec![
                section::Row { id: "on".into(), kind: 0, options: vec![], min: None, max: None, step: None, default: Some(1.0), hint: None, unit: None, decimals: 0, url: None },
                section::Row { id: "shape".into(), kind: 1, options: vec!["round".into(), "flat".into()], min: None, max: None, step: None, default: None, hint: Some("shape_hint".into()), unit: None, decimals: 0, url: None },
                section::Row { id: "about".into(), kind: 8, options: vec![], min: None, max: None, step: None, default: None, hint: None, unit: None, decimals: 0, url: None },
            ],
            menu: vec![],
            places: vec![],
            devices: vec![],
            later: None,
        };
        let p = web_plugin(&section, &[], "name = \"Orbit\"\nround = \"Round\"\nshape_hint = \"How\"\n", false);
        assert_eq!((p.name.as_str(), p.about), ("Orbit", None));
        let row = |i: usize| &p.rows[i];
        assert_eq!((row(0).kind, row(0).numeric, row(0).label.as_str()), ("Switch", true, "on"));
        assert_eq!(row(1).options, ["Round", "flat"]);
        assert_eq!(row(1).hint.as_deref(), Some("How"));
        assert!(!row(2).numeric);
    }
}
