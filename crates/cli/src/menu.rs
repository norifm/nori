//! The menu of what is selected: a song's as the core lists it (`menus::song_menu`, its less used actions
//! under More), an album's or a playlist's, and on a page the page's own actions after the song's.

use nori_core::client::Starrable;
use nori_core::menus::{JamOffer, PageAction, SongAction, SongDownload};
use nori_core::Song;

use crate::app::{App, Cmd, Item, Load, Overlay, Page, Panel, Sel, Target};
use crate::backend::Fetch;

/// A line of a menu.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuLine {
    /// The name of what the lines under it act on.
    Heading(String),
    Act(MenuAct),
    /// Unfolds the song's less used actions in its place.
    More,
}

/// What a menu line does.
#[derive(Debug, Clone, PartialEq)]
pub enum MenuAct {
    Song(SongAction),
    /// An album's or a playlist's.
    Play,
    PlayNext,
    AddToQueue,
    /// The page's.
    PlayPage,
    ShufflePage,
    SaveMix,
    Page(PageAction),
}

/// Song actions this client has no screen or work for: they are not listed.
fn offered(a: &SongAction) -> bool {
    !matches!(a, SongAction::StopDownload | SongAction::AddToLibrary | SongAction::SleepTimer | SongAction::Share | SongAction::Details)
}

impl App {
    /// Opens the menu of the song playing (from the full player), or of what is selected.
    pub fn open_menu(&mut self) {
        let from_player = self.full && matches!(self.panel, Panel::Playing | Panel::Lyrics);
        let item = if from_player { self.song.clone().map(|s| Item::Song(vec![s], 0)) } else { self.selected() };
        let menu = match item {
            Some(Item::Song(songs, at)) => {
                let song = songs[at].clone();
                let mut lines = self.song_lines(&song, from_player);
                let folded = split_more(&mut lines);
                if !from_player {
                    lines.extend(self.page_lines());
                }
                Some(Overlay::Menu { title: song.title.clone(), item: Box::new(Item::Song(vec![song], 0)), lines, folded, sel: Sel::default() })
            }
            Some(item @ (Item::Album(_) | Item::Playlist(_))) => {
                let title = match &item {
                    Item::Album(a) => a.name.clone(),
                    Item::Playlist(p) => p.name.clone(),
                    _ => String::new(),
                };
                // A jam guest only asks for songs.
                let play = (!self.guest()).then_some(MenuLine::Act(MenuAct::Play));
                let lines = play.into_iter().chain([MenuLine::Act(MenuAct::PlayNext), MenuLine::Act(MenuAct::AddToQueue)]).collect();
                Some(Overlay::Menu { title, item: Box::new(item), lines, folded: Vec::new(), sel: Sel::default() })
            }
            _ => None,
        };
        match menu {
            Some(m) => self.overlay = Some(m),
            None => self.dirty = false,
        }
    }

    /// The core's menu of `song`, without what this client cannot do.
    fn song_lines(&self, song: &Song, player: bool) -> Vec<MenuLine> {
        let starred = self.starred(Starrable::Song, &song.id, song.starred);
        let download = self.downloaded.get(&song.id).copied().unwrap_or(SongDownload::None);
        let lyrics = self.lyrics.as_ref().is_some_and(|l| !l.pick.lyrics.lines.is_empty()) && self.lyrics_for.as_deref() == Some(&song.id);
        let jam = if self.guest() { Some(JamOffer::Guest) } else if self.devices.jams && self.devices.jam.is_none() { Some(JamOffer::Start) } else { None };
        let r = &self.rules;
        let items = nori_core::menus::song_menu(song.clone(), starred, download, player, lyrics, r.stars, r.account, r.downloads, r.radio, jam);
        let mut lines = Vec::with_capacity(items.len() + 1);
        let mut more = false;
        for i in items.into_iter().filter(|i| offered(&i.action) && i.enabled) {
            if i.more && !more {
                lines.push(MenuLine::More);
                more = true;
            }
            lines.push(MenuLine::Act(MenuAct::Song(i.action)));
        }
        lines
    }

    /// The page's own actions, under its name: Play, Shuffle, a mix's Save as playlist, then its head's.
    fn page_lines(&self) -> Vec<MenuLine> {
        let Some(page) = self.page() else { return Vec::new() };
        let title = match page {
            Page::Album { detail: Load::Ready(d), .. } | Page::DownloadedAlbum { detail: Load::Ready(d), .. } => d.album.name.clone(),
            Page::Playlist { detail: Load::Ready(d), .. } => d.playlist.name.clone(),
            Page::Artist { detail: Load::Ready(d), .. } => d.artist.name.clone(),
            Page::Mix { detail: Load::Ready(m), .. } => crate::text::mix_name(m.name).into(),
            _ => return Vec::new(),
        };
        let mut lines = vec![MenuLine::Heading(title), MenuLine::Act(MenuAct::PlayPage), MenuLine::Act(MenuAct::ShufflePage)];
        if matches!(page, Page::Mix { .. }) && self.rules.account {
            lines.push(MenuLine::Act(MenuAct::SaveMix));
        }
        lines.extend(self.page_actions().into_iter().filter(|a| *a != PageAction::Edit).map(|a| MenuLine::Act(MenuAct::Page(a))));
        lines
    }

    /// Does what the menu's line does on `item`, and closes the menu.
    pub fn menu_act(&mut self, act: MenuAct, item: Item) {
        self.overlay = None;
        let song = match &item {
            Item::Song(songs, at) => songs.get(*at).cloned(),
            _ => None,
        };
        let fetch = match &item {
            Item::Album(a) => Some(Fetch::Album(a.id.clone())),
            Item::Playlist(p) => Some(Fetch::Playlist(p.id.clone())),
            _ => None,
        };
        match (act, song) {
            (MenuAct::Play, _) => self.cmds.extend(fetch.map(|f| Cmd::PlayFetch(f, false))),
            (MenuAct::PlayNext, _) => self.cmds.extend(fetch.map(|f| Cmd::EnqueueFetch(f, true))),
            (MenuAct::AddToQueue, _) => self.cmds.extend(fetch.map(|f| Cmd::EnqueueFetch(f, false))),
            (MenuAct::PlayPage, _) => self.play_page(false),
            (MenuAct::ShufflePage, _) => self.play_page(true),
            (MenuAct::SaveMix, _) => self.save_mix(),
            (MenuAct::Page(a), _) => self.page_act(a),
            (MenuAct::Song(_), None) => {}
            (MenuAct::Song(a), Some(s)) => self.song_act(a, s),
        }
    }

    fn song_act(&mut self, a: SongAction, s: Song) {
        match a {
            SongAction::Favourite { .. } => self.star_song(s),
            SongAction::PlayNext => self.cmds.push(Cmd::Enqueue(vec![s], true)),
            SongAction::AddToQueue => self.cmds.push(Cmd::Enqueue(vec![s], false)),
            SongAction::AddToPlaylist => self.pick_playlist(vec![s.id]),
            SongAction::RemoveDownload => self.cmds.push(Cmd::DownloadRemove(s.id)),
            SongAction::Download => self.cmds.push(Cmd::Download(vec![s])),
            SongAction::GoToAlbum { id } => self.open_album(id),
            SongAction::GoToArtist { id, .. } => self.open_artist(id),
            SongAction::Lyrics => self.set_panel(Panel::Lyrics),
            SongAction::StartRadio => self.cmds.push(Cmd::PlayFetch(Fetch::Radio(s.id), false)),
            SongAction::InstantMix => self.cmds.push(Cmd::PlayFetch(Fetch::InstantMix(s.id), false)),
            SongAction::ExcludeFromMixes => self.cmds.push(Cmd::ExcludeFromMixes(s.id)),
            SongAction::StartJam => self.jam_with(Some(Item::Song(vec![s], 0))),
            SongAction::StopDownload | SongAction::AddToLibrary | SongAction::SleepTimer | SongAction::Share | SongAction::Details => {}
        }
    }

    /// The account's playlists to add `songs` to, a new one first.
    fn pick_playlist(&mut self, songs: Vec<String>) {
        let mut options = vec![(crate::text::NEW_PLAYLIST.to_string(), String::new())];
        options.extend(self.library.playlists.ready().into_iter().flatten().map(|p| (p.name.clone(), p.id.clone())));
        if self.library.playlists.ready().is_none() {
            self.library.playlists = Load::Loading;
            self.cmds.push(Cmd::Load(crate::backend::Req::Playlists));
        }
        self.overlay = Some(Overlay::Picker { title: crate::text::ADD_TO_PLAYLIST.into(), options, sel: Sel::default(), target: Target::Playlist(songs) });
    }
}

/// Takes the lines after More out of `lines`, to unfold in its place.
fn split_more(lines: &mut Vec<MenuLine>) -> Vec<MenuLine> {
    match lines.iter().position(|l| *l == MenuLine::More) {
        Some(at) => lines.split_off(at + 1),
        None => Vec::new(),
    }
}
