//! A plugin repo's `index.json`, and what stops a plugin running in a browser: pure, with no plugin
//! host, so the web client reads it too. [`web_blocks`] is the one rule, for a listing and an install.

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
    /// Requests to sites that don't allow a web page's.
    OtherSites,
}

/// The name of the host interface in a component's import (`nori:plugin/sockets@2.0.0`).
fn interface(import: &str) -> Option<&str> {
    let name = import.strip_prefix("nori:plugin/")?;
    Some(name.split('@').next().unwrap_or(name))
}

/// What stops a plugin running in a browser, from what it fills, the hosts it names and the host
/// interfaces it imports (as [`imports`] names them), each reason once; none: it can.
pub fn web_blocks(points: &[String], net: &[String], imports: &[String]) -> Vec<WebBlock> {
    use WebBlock::*;
    let mut blocks = Vec::new();
    let mut add = |b: WebBlock| {
        if !blocks.contains(&b) {
            blocks.push(b);
        }
    };
    if points.iter().any(|p| p == "device-control") {
        add(DeviceControl);
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
            "http" if net.iter().any(|h| h != "lan") => add(OtherSites),
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
pub fn web_fit(listed: &Listed) -> WebFit {
    let imports = listed.imports.as_deref();
    let blocks = web_blocks(&listed.points, &listed.net, imports.unwrap_or_default());
    match (blocks.is_empty(), imports) {
        (false, _) => WebFit::Blocked(blocks),
        (true, Some(_)) => WebFit::Supported,
        (true, None) => WebFit::Unknown,
    }
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
        for (points, net, imports, want) in [
            (vec!["sound-stage"], vec![], vec!["log", "store"], vec![]),
            (vec!["lifecycle"], vec!["lan"], vec!["sockets", "discovery", "cast", "audio-out"], vec![LocalNetwork, Sockets, Discovery, Casting]),
            (vec!["device-control"], vec![], vec!["bluetooth"], vec![DeviceControl, Bluetooth]),
            (vec!["provider"], vec!["music.youtube.com"], vec!["http", "sign-in", "wake"], vec![OtherSites, SignIn, Wake]),
            (vec!["lifecycle"], vec!["lan"], vec!["http"], vec![LocalNetwork]),
            (vec!["lifecycle"], vec![], vec!["cast", "audio-out", "media"], vec![Casting]),
        ] {
            assert_eq!(web_blocks(&all(&points), &all(&net), &all(&imports)), want, "{points:?} {net:?} {imports:?}");
        }
    }

    #[test]
    fn a_listing_is_called_supported_only_when_it_says_what_it_imports() {
        let listed = |points: &[&str], imports: Option<&[&str]>| Listed { id: "a".into(), name: "A".into(), about: None, source: None, readme: None, icon: None, points: all(points), imports: imports.map(all), net: Vec::new(), versions: Vec::new() };
        assert_eq!(web_fit(&listed(&["lifecycle"], None)), WebFit::Unknown);
        assert_eq!(web_fit(&listed(&["lifecycle"], Some(&["log"]))), WebFit::Supported);
        assert_eq!(web_fit(&listed(&["device-control"], None)), WebFit::Blocked(vec![DeviceControl]), "what is known stops it already");
        assert_eq!(web_fit(&listed(&["lifecycle"], Some(&["sockets", "bluetooth"]))), WebFit::Blocked(vec![Sockets, Bluetooth]));
    }

    #[test]
    fn an_index_reads_with_or_without_what_a_plugin_imports() {
        let text = br#"{"plugins":[{"id":"a","name":"A","versions":[{"version":"1","api":2,"url":"a","sha256":"0"}]},{"id":"b","name":"B","imports":["http"],"net":["x.example"],"versions":[]}]}"#;
        let index = parse(text).unwrap();
        assert_eq!(index.plugins[0].imports, None);
        assert_eq!(index.plugins[1].imports.as_deref(), Some(&["http".to_string()][..]));
        assert!(parse(b"[]").is_err());
    }
}
