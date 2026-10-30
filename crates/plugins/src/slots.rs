//! The places in nori's screens plugins fill, as a client asks for them: the actions a plugin adds to
//! its menus, lines in a song's info, the player's sleeve, a line under the player's output, and the
//! rows it shows on the devices sheet ("Playing on"). nori draws them all; a plugin only answers. Each asks only the
//! enabled plugins whose manifest names that place, in their order, and never makes one's instance
//! before a screen asks.

use nori_model::{Album, Artist, Playlist, Song};

use crate::convert::{album_out, artist_out, playlist_out, song_out};
use crate::manifest::{Glyph, ItemKind, Place};
use crate::runtime::points::slots as w;
use crate::runtime::types as t;
use crate::view::PluginCard;
use crate::Plugins;

/// An action a plugin adds to the menus of an item kind.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MenuEntry {
    pub plugin: String,
    pub action: String,
    /// In the plugin's words.
    pub label: String,
    pub glyph: Glyph,
}

/// The items a menu action runs on.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Items {
    Songs(Vec<Song>),
    Albums(Vec<Album>),
    Artists(Vec<Artist>),
    Playlists(Vec<Playlist>),
}

/// A plugin's lines in a song's info.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InfoGroup {
    pub plugin: String,
    /// The plugin's name, over its lines.
    pub name: String,
    pub lines: Vec<InfoLine>,
}

#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct InfoLine {
    pub words: String,
    pub link: Option<String>,
}

/// The player's sleeve from a plugin.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Artwork {
    pub plugin: String,
    /// A muted looping video (HLS or MP4) when true, else a still image.
    pub looping: bool,
    pub url: String,
    /// A loop's first frame: an image URL with `{size}` where the pixels go.
    pub poster: Option<String>,
}

impl Plugins {
    /// The running plugins that fill `place`, in their order.
    fn filling(&self, place: Place) -> Vec<String> {
        self.running().into_iter().filter(|i| i.manifest.extends("slots") && i.manifest.places.contains(&place)).map(|i| i.manifest.id.clone()).collect()
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Plugins {
    /// The actions plugins add to the menus of `kind`, in `lang`: from their manifests, with no call.
    pub fn menu_entries(&self, kind: ItemKind, lang: String) -> Vec<MenuEntry> {
        self.running()
            .into_iter()
            .filter(|i| i.manifest.extends("slots"))
            .flat_map(|i| {
                let words = crate::view::words(&self.folder(&i.manifest.id), &lang);
                i.manifest
                    .menu
                    .iter()
                    .filter(|a| a.kinds.contains(&kind))
                    .map(|a| MenuEntry { plugin: i.manifest.id.clone(), action: a.id.clone(), label: words.get(&a.id).cloned().unwrap_or_else(|| a.id.clone()), glyph: a.glyph })
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// Runs `entry`'s action on `items`.
    pub async fn run_menu(&self, entry: MenuEntry, items: Items) {
        let Some(w) = self.worker(&entry.plugin) else { return };
        let items: Vec<t::Item> = match items {
            Items::Songs(v) => v.iter().map(|s| t::Item::Song(song_out(s))).collect(),
            Items::Albums(v) => v.iter().map(|a| t::Item::Album(album_out(a))).collect(),
            Items::Artists(v) => v.iter().map(|a| t::Item::Artist(artist_out(a))).collect(),
            Items::Playlists(v) => v.iter().map(|p| t::Item::Playlist(playlist_out(p))).collect(),
        };
        w.call(move |s, p| p.nori_plugin_slots().call_menu(s, &entry.action, &items)).await;
    }

    /// The lines plugins add to `song`'s info, each plugin's under its name in `lang`.
    pub async fn song_info(&self, song: Song, lang: String) -> Vec<InfoGroup> {
        let mut out = Vec::new();
        for id in self.filling(Place::SongInfo) {
            let Some(w) = self.worker(&id) else { continue };
            let s = song_out(&song);
            let lines = w.call(move |st, p| p.nori_plugin_slots().call_song_info(st, &s)).await.unwrap_or_default();
            if !lines.is_empty() {
                let name = crate::view::words(&self.folder(&id), &lang).remove("name").unwrap_or_else(|| id.clone());
                out.push(InfoGroup { plugin: id, name, lines: lines.into_iter().map(|l| InfoLine { words: l.words, link: l.link }).collect() });
            }
        }
        out
    }

    /// The player's sleeve for `song` from the first plugin that has one; None keeps nori's cover.
    pub async fn artwork(&self, song: Song) -> Option<Artwork> {
        for id in self.filling(Place::Artwork) {
            let Some(w) = self.worker(&id) else { continue };
            let s = song_out(&song);
            if let Some(sleeve) = w.call(move |st, p| p.nori_plugin_slots().call_artwork(st, &s)).await.flatten() {
                return Some(match sleeve {
                    w::Sleeve::Still(url) => Artwork { plugin: id, looping: false, url, poster: None },
                    w::Sleeve::Loop(m) => Artwork { plugin: id, looping: true, url: m.video, poster: m.poster },
                });
            }
        }
        None
    }

    /// `artwork`'s address answered HTTP `status`: its plugin may look it up again.
    pub fn artwork_failed(&self, artwork: Artwork, song: Song, status: u16) {
        if let Some(w) = self.worker(&artwork.plugin) {
            let s = song_out(&song);
            w.post(move |st, p| p.nori_plugin_slots().call_artwork_failed(st, &s, status));
        }
    }

    /// The lines plugins say under the player's output row.
    pub async fn player_status(&self) -> Vec<String> {
        let mut out = Vec::new();
        for id in self.filling(Place::PlayerStatus) {
            let Some(w) = self.worker(&id) else { continue };
            out.extend(w.call(|s, p| p.nori_plugin_slots().call_player_status(s)).await.flatten());
        }
        out
    }

    /// The plugins whose rows show on the devices sheet ("Playing on") too, in `lang`.
    pub fn device_cards(&self, lang: String) -> Vec<PluginCard> {
        let shown = self.filling(Place::Devices);
        self.cards(lang).into_iter().filter(|c| shown.contains(&c.id)).collect()
    }
}
