//! WEB_REMIX answers into plain records. Defensive: an item missing what it needs is dropped, not the page.

use serde_json::Value;

/// A link to an artist's channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistLink {
    /// Channel id (`UC...`).
    pub id: String,
    pub name: String,
}

/// A link to an album.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumLink {
    /// Browse id (`MPREb_...`).
    pub id: String,
    pub name: String,
}

/// A song (a video that plays).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Song {
    pub video_id: String,
    pub title: String,
    /// The artists as YouTube writes them ("A & B"); empty when it does not say.
    pub artist: String,
    /// The artists it links to, in order.
    pub artists: Vec<ArtistLink>,
    pub album: Option<AlbumLink>,
    /// Seconds; 0 when not shown.
    pub duration: u32,
    /// Position on its album, from 1; 0 outside an album page.
    pub track: u32,
    /// 0 when not known.
    pub year: u32,
    /// Thumbnail base URL ([`thumbnail_url`] sizes it).
    pub thumbnail: Option<String>,
}

/// An album, single or EP.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Album {
    /// Browse id (`MPREb_...`).
    pub id: String,
    pub title: String,
    /// The artists as YouTube writes them.
    pub artist: String,
    /// The artists it links to, in order.
    pub artists: Vec<ArtistLink>,
    /// 0 when not shown.
    pub year: u32,
    /// "Album", "Single", "EP", ... as shown; None when not shown.
    pub release_type: Option<String>,
    pub thumbnail: Option<String>,
}

/// An artist's channel.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Artist {
    /// Channel id (`UC...`).
    pub id: String,
    pub name: String,
    pub thumbnail: Option<String>,
}

/// A playlist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Playlist {
    /// Browse id (`VL...`).
    pub id: String,
    pub title: String,
    pub description: Option<String>,
    /// Who made it, as shown.
    pub author: Option<String>,
    /// Songs in it as its header counts them; None when it does not say.
    pub song_count: Option<u32>,
    pub thumbnail: Option<String>,
}

/// Songs, albums and artists of a search, in YouTube's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SearchResults {
    pub songs: Vec<Song>,
    pub albums: Vec<Album>,
    pub artists: Vec<Artist>,
}

/// An album and its songs, which carry the album's name, year and, where they lack their own, its
/// thumbnail and artist.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AlbumPage {
    pub album: Album,
    pub songs: Vec<Song>,
}

/// An artist, its albums and singles, its top songs, similar artists and description.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtistPage {
    pub artist: Artist,
    pub albums: Vec<Album>,
    pub top_songs: Vec<Song>,
    pub similar: Vec<Artist>,
    pub description: Option<String>,
}

/// A playlist and the songs of its first page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaylistPage {
    pub playlist: Playlist,
    pub songs: Vec<Song>,
}

/// One titled row of a feed (the home page's "Quick picks", "New releases", ...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Item>,
}

/// One song, album, artist or playlist of a list or shelf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Song(Song),
    Album(Album),
    Artist(Artist),
    Playlist(Playlist),
}

/// Where a browse link leads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leads {
    Album,
    Artist,
    Playlist,
    Elsewhere,
}

fn leads(page_type: &str) -> Leads {
    match page_type {
        "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => Leads::Album,
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" => Leads::Artist,
        "MUSIC_PAGE_TYPE_PLAYLIST" => Leads::Playlist,
        _ => Leads::Elsewhere,
    }
}

/// One run of text, and the page or video it links to.
struct Run<'a> {
    text: &'a str,
    browse: Option<(&'a str, Leads)>,
    video: Option<&'a str>,
}

fn str_at<'a>(v: &'a Value, pointer: &str) -> Option<&'a str> {
    v.pointer(pointer).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// The browse link of an endpoint holder (a run or an item).
fn browse(v: &Value) -> Option<(&str, Leads)> {
    let b = v.pointer("/navigationEndpoint/browseEndpoint")?;
    let page = str_at(b, "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType").unwrap_or("");
    Some((str_at(b, "/browseId")?, leads(page)))
}

/// The runs of a text object (`{"runs": [...]}`).
fn runs(text: &Value) -> Vec<Run<'_>> {
    let Some(list) = text.get("runs").and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .map(|r| Run { text: r.get("text").and_then(Value::as_str).unwrap_or(""), browse: browse(r), video: str_at(r, "/navigationEndpoint/watchEndpoint/videoId") })
        .collect()
}

fn joined(runs: &[Run]) -> String {
    runs.iter().map(|r| r.text).collect()
}

/// Runs split at YouTube's " • " separators.
fn segments<'a, 'b>(runs: &'b [Run<'a>]) -> impl Iterator<Item = &'b [Run<'a>]> {
    runs.split(|r| r.text.trim() == "•").filter(|s| !s.is_empty())
}

/// Seconds of "3:08" or "1:02:03".
fn seconds(text: &str) -> Option<u32> {
    let parts: Vec<&str> = text.trim().split(':').collect();
    if parts.len() < 2 || parts.len() > 3 || parts.iter().any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    parts.iter().try_fold(0u32, |total, p| Some(total * 60 + p.parse::<u32>().ok()?))
}

/// A year standing alone ("2013").
fn year(text: &str) -> Option<u32> {
    let t = text.trim();
    (t.len() == 4 && t.bytes().all(|b| b.is_ascii_digit())).then(|| t.parse().ok()).flatten()
}

/// Whether a thumbnail URL takes its size after `=` (Google's image servers).
fn sized_host(url: &str) -> bool {
    url.split('/').nth(2).is_some_and(|h| h.ends_with(".googleusercontent.com") || h.ends_with(".ggpht.com"))
}

/// The base URL of a thumbnail object: the largest one's URL, without its size where it takes one.
fn thumbnail(v: &Value) -> Option<String> {
    let list = v.pointer("/musicThumbnailRenderer/thumbnail/thumbnails").or_else(|| v.pointer("/thumbnails"))?.as_array()?;
    let url = list.last()?.get("url")?.as_str()?;
    Some(match url.rsplit_once('=') {
        Some((base, _)) if sized_host(url) => base.to_string(),
        _ => url.to_string(),
    })
}

/// The URL to fetch thumbnail `base` at `px` square: Google's image servers scale it, other URLs (video
/// stills) come as they are.
pub fn thumbnail_url(base: &str, px: u32) -> String {
    if sized_host(base) {
        format!("{base}=w{px}-h{px}-l90-rj")
    } else {
        base.to_string()
    }
}

/// The artists a segment links to, and the segment's text as the display string.
fn artists_of(segment: &[Run]) -> Option<(String, Vec<ArtistLink>)> {
    let links: Vec<ArtistLink> = segment.iter().filter_map(|r| match r.browse {
        Some((b, Leads::Artist)) => Some(ArtistLink { id: b.to_string(), name: r.text.to_string() }),
        _ => None,
    }).collect();
    (!links.is_empty()).then(|| (joined(segment), links))
}

/// The text runs of a list item's flex and fixed columns, in order.
fn columns(item: &Value) -> Vec<Vec<Run<'_>>> {
    fn column<'a>(c: &'a Value, key: &str) -> Option<Vec<Run<'a>>> {
        c.get(key).and_then(|r| r.get("text")).map(runs)
    }
    let flex = item.get("flexColumns").and_then(Value::as_array).into_iter().flatten().filter_map(|c| column(c, "musicResponsiveListItemFlexColumnRenderer"));
    let fixed = item.get("fixedColumns").and_then(Value::as_array).into_iter().flatten().filter_map(|c| column(c, "musicResponsiveListItemFixedColumnRenderer"));
    flex.chain(fixed).collect()
}

/// A `musicResponsiveListItemRenderer`: a song when it plays a video, else what it links to.
fn list_item(item: &Value) -> Option<Item> {
    let cols = columns(item);
    let title = joined(cols.first()?);
    let thumb = item.get("thumbnail").and_then(thumbnail);
    let video = str_at(item, "/playlistItemData/videoId").or_else(|| cols[0].iter().find_map(|r| r.video));
    if let Some(v) = video {
        let mut song = Song { video_id: v.to_string(), title, thumbnail: thumb, ..Default::default() };
        for (i, col) in cols.iter().enumerate().skip(1) {
            for (k, seg) in segments(col).enumerate() {
                if let Some((name, links)) = artists_of(seg).filter(|_| song.artists.is_empty()) {
                    song.artist = name;
                    song.artists = links;
                } else if let Some((b, Leads::Album)) = seg.iter().find_map(|r| r.browse) {
                    song.album = Some(AlbumLink { id: b.to_string(), name: joined(seg) });
                } else if let Some(s) = seconds(&joined(seg)) {
                    song.duration = s;
                } else if i == 1 && k == 0 && song.artist.is_empty() {
                    song.artist = joined(seg);
                }
            }
        }
        return Some(Item::Song(song));
    }
    let (b, kind) = browse(item)?;
    linked(kind, b, title, thumb, cols.get(1).map_or(&[], Vec::as_slice))
}

/// A card (`musicTwoRowItemRenderer`): a song when it plays a video, else the album, artist or
/// playlist it links to.
fn card(item: &Value) -> Option<Item> {
    let title = joined(&runs(item.get("title")?));
    let subtitle = item.get("subtitle").map(runs).unwrap_or_default();
    let thumb = item.get("thumbnailRenderer").and_then(thumbnail);
    if let Some(v) = str_at(item, "/navigationEndpoint/watchEndpoint/videoId") {
        let mut song = Song { video_id: v.to_string(), title, thumbnail: thumb, ..Default::default() };
        if let Some((name, links)) = segments(&subtitle).find_map(artists_of) {
            song.artist = name;
            song.artists = links;
        }
        return Some(Item::Song(song));
    }
    let (b, kind) = browse(item)?;
    linked(kind, b, title, thumb, &subtitle)
}

/// The album or artist a link to `browse_id` (`kind`) shows, titled `title`, with `subtitle`'s artists,
/// year and release type.
fn linked(kind: Leads, browse_id: &str, title: String, thumb: Option<String>, subtitle: &[Run]) -> Option<Item> {
    Some(match kind {
        Leads::Album => {
            let mut a = Album { id: browse_id.to_string(), title, thumbnail: thumb, ..Default::default() };
            for (k, seg) in segments(subtitle).enumerate() {
                let text = joined(seg);
                if let Some((name, links)) = artists_of(seg) {
                    a.artist = name;
                    a.artists = links;
                } else if let Some(y) = year(&text) {
                    a.year = y;
                } else if k == 0 {
                    a.release_type = Some(text);
                } else if a.artist.is_empty() {
                    a.artist = text;
                }
            }
            Item::Album(a)
        }
        Leads::Artist => Item::Artist(Artist { id: browse_id.to_string(), name: title, thumbnail: thumb }),
        Leads::Playlist => Item::Playlist(Playlist { id: browse_id.to_string(), title, description: Some(joined(subtitle)).filter(|d| !d.is_empty()), thumbnail: thumb, ..Default::default() }),
        Leads::Elsewhere => return None,
    })
}

/// Every list item in a shelf's `contents`.
fn shelf_items(shelf: &Value) -> impl Iterator<Item = Item> + '_ {
    shelf.get("contents").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("musicResponsiveListItemRenderer")).filter_map(list_item)
}

/// The sections of a browse or search answer at `pointer`.
fn sections<'a>(v: &'a Value, pointer: &str) -> impl Iterator<Item = &'a Value> {
    v.pointer(pointer).and_then(Value::as_array).into_iter().flatten()
}

/// A search answer's songs, albums and artists, in YouTube's order.
pub(crate) fn search(v: &Value) -> SearchResults {
    let mut found = SearchResults::default();
    for s in sections(v, "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents") {
        for item in s.get("musicShelfRenderer").into_iter().flat_map(shelf_items) {
            match item {
                Item::Song(s) => found.songs.push(s),
                Item::Album(a) => found.albums.push(a),
                Item::Artist(a) => found.artists.push(a),
                Item::Playlist(_) => {}
            }
        }
    }
    found
}

/// The header of a two-column browse page (album, playlist).
fn responsive_header(v: &Value) -> Option<&Value> {
    sections(v, "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents").find_map(|s| s.get("musicResponsiveHeaderRenderer"))
}

/// The songs of a two-column browse page's first shelf named `shelf`.
fn listed_songs(v: &Value, shelf: &str) -> Vec<Song> {
    sections(v, "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents")
        .filter_map(|s| s.get(shelf))
        .flat_map(shelf_items)
        .filter_map(|i| match i {
            Item::Song(s) => Some(s),
            _ => None,
        })
        .collect()
}

/// An album page (`browse` of `MPREb_...`, which is `id`).
pub(crate) fn album(v: &Value, id: &str) -> AlbumPage {
    let header = responsive_header(v).unwrap_or(&Value::Null);
    let mut album = Album { id: id.to_string(), title: header.get("title").map(|t| joined(&runs(t))).unwrap_or_default(), thumbnail: header.get("thumbnail").and_then(thumbnail), ..Default::default() };
    if let Some((name, links)) = header.get("straplineTextOne").map(runs).as_deref().and_then(artists_of) {
        album.artist = name;
        album.artists = links;
    }
    let subtitle = header.get("subtitle").map(runs).unwrap_or_default();
    for (k, seg) in segments(&subtitle).enumerate() {
        match year(&joined(seg)) {
            Some(y) => album.year = y,
            None if k == 0 => album.release_type = Some(joined(seg)),
            None => {}
        }
    }
    let mut songs = listed_songs(v, "musicShelfRenderer");
    for (i, s) in songs.iter_mut().enumerate() {
        s.track = i as u32 + 1;
        s.album = Some(AlbumLink { id: album.id.clone(), name: album.title.clone() });
        s.year = album.year;
        if s.thumbnail.is_none() {
            s.thumbnail = album.thumbnail.clone();
        }
        if s.artist.is_empty() {
            s.artist = album.artist.clone();
            s.artists = album.artists.clone();
        }
    }
    AlbumPage { album, songs }
}

/// An artist page (`browse` of `UC...`, which is `id`). Albums without artists of their own are the
/// artist's.
pub(crate) fn artist(v: &Value, id: &str) -> ArtistPage {
    let header = v.get("header").and_then(|h| h.get("musicImmersiveHeaderRenderer").or_else(|| h.get("musicVisualHeaderRenderer"))).unwrap_or(&Value::Null);
    let name = header.get("title").map(|t| joined(&runs(t))).unwrap_or_default();
    let mut page = ArtistPage { artist: Artist { id: id.to_string(), name, thumbnail: header.get("thumbnail").and_then(thumbnail) }, ..Default::default() };
    for s in sections(v, "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents") {
        if let Some(shelf) = s.get("musicShelfRenderer") {
            page.top_songs.extend(shelf_items(shelf).filter_map(|i| match i {
                Item::Song(s) => Some(s),
                _ => None,
            }));
        } else if let Some(shelf) = s.get("musicCarouselShelfRenderer") {
            for item in shelf.get("contents").and_then(Value::as_array).into_iter().flatten().filter_map(|c| c.get("musicTwoRowItemRenderer")).filter_map(card) {
                match item {
                    Item::Album(mut a) if !page.albums.iter().any(|b| b.id == a.id) => {
                        if a.artist.is_empty() {
                            a.artist = page.artist.name.clone();
                            a.artists = vec![ArtistLink { id: page.artist.id.clone(), name: page.artist.name.clone() }];
                        }
                        page.albums.push(a);
                    }
                    Item::Artist(a) => page.similar.push(a),
                    _ => {}
                }
            }
        } else if let Some(about) = s.get("musicDescriptionShelfRenderer") {
            page.description = about.get("description").map(|d| joined(&runs(d))).filter(|b| !b.trim().is_empty());
        }
    }
    page
}

/// A playlist page (`browse` of `VL...`, which is `id`).
pub(crate) fn playlist(v: &Value, id: &str) -> PlaylistPage {
    let header = responsive_header(v).unwrap_or(&Value::Null);
    let songs = listed_songs(v, "musicPlaylistShelfRenderer");
    let counted = header.get("secondSubtitle").map(runs).and_then(|r| r.first().and_then(|r| r.text.split_whitespace().next()).map(|n| n.replace(',', "")).and_then(|n| n.parse().ok()));
    let playlist = Playlist {
        id: id.to_string(),
        title: header.get("title").map(|t| joined(&runs(t))).unwrap_or_default(),
        description: header.pointer("/description/musicDescriptionShelfRenderer/description").map(|d| joined(&runs(d))).filter(|d| !d.is_empty()),
        author: header.get("straplineTextOne").map(|t| joined(&runs(t))).filter(|o| !o.is_empty()),
        song_count: counted,
        thumbnail: header.get("thumbnail").and_then(thumbnail),
    };
    PlaylistPage { playlist, songs }
}

/// A carousel's title and its songs, albums, artists and playlists, in order.
fn carousel(shelf: &Value) -> Option<Shelf> {
    let title = joined(&runs(shelf.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title")?));
    let items = shelf
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("musicResponsiveListItemRenderer").and_then(list_item).or_else(|| c.get("musicTwoRowItemRenderer").and_then(card)))
        .collect::<Vec<_>>();
    (!title.is_empty() && !items.is_empty()).then_some(Shelf { title, items })
}

/// A feed page (`browse` of `FEmusic_home`, or its continuation): its shelves, and the token of the
/// page after it.
pub(crate) fn feed(v: &Value) -> (Vec<Shelf>, Option<String>) {
    let list = v
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer")
        .or_else(|| v.pointer("/continuationContents/sectionListContinuation"))
        .unwrap_or(&Value::Null);
    let shelves = sections(list, "/contents").filter_map(|s| s.get("musicCarouselShelfRenderer")).filter_map(carousel).collect();
    (shelves, str_at(list, "/continuations/0/nextContinuationData/continuation").map(str::to_string))
}

/// A library page (`browse` of `FEmusic_liked_playlists`, `FEmusic_liked_albums`, ...): the cards of its
/// grid, or the items of its list.
pub(crate) fn library(v: &Value) -> Vec<Item> {
    let mut items = Vec::new();
    for s in sections(v, "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents") {
        // The grid stands alone or inside an item section.
        let inner = s.pointer("/itemSectionRenderer/contents").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
        for holder in std::iter::once(s).chain(inner) {
            if let Some(grid) = holder.pointer("/gridRenderer/items").and_then(Value::as_array) {
                items.extend(grid.iter().filter_map(|c| c.get("musicTwoRowItemRenderer")).filter_map(card));
            }
            if let Some(shelf) = holder.get("musicShelfRenderer") {
                items.extend(shelf_items(shelf));
            }
        }
    }
    items
}

/// Whether an answer was given to a visitor without an account (`logged_in` 0).
pub(crate) fn logged_out(v: &Value) -> bool {
    v.pointer("/responseContext/serviceTrackingParams")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("params").and_then(Value::as_array))
        .flatten()
        .any(|p| p.get("key").and_then(Value::as_str) == Some("logged_in") && p.get("value").and_then(Value::as_str) == Some("0"))
}

/// The signed-in account's name, as its menu shows it.
pub(crate) fn account_name(v: &Value) -> Option<String> {
    let header = v.pointer("/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer")?;
    Some(joined(&runs(header.get("accountName")?))).filter(|n| !n.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::fixture;

    fn json(name: &str) -> Value {
        serde_json::from_str(&fixture(name)).unwrap()
    }

    const DAFT_PUNK: &str = "UCRr1xG_2WIDs18a6cIiCxeA";

    #[test]
    fn search_songs() {
        let found = search(&json("search_songs"));
        assert!(found.albums.is_empty() && found.artists.is_empty());
        let s = &found.songs[0];
        assert_eq!((s.video_id.as_str(), s.title.as_str(), s.artist.as_str()), ("0wTOi2TCSSk", "Robot Rock (Edit)", "Daft Punk"));
        assert_eq!(s.artists, [ArtistLink { id: DAFT_PUNK.into(), name: "Daft Punk".into() }]);
        assert_eq!(s.album, Some(AlbumLink { id: "MPREb_NP5KiWDRCje".into(), name: "Robot Rock (Edit)".into() }));
        assert_eq!(s.duration, 188);
        assert_eq!(s.thumbnail.as_deref(), Some("https://yt3.googleusercontent.com/UH71RzkuVtjABf5KsL72KN32LlWBidolaYW1Cyp2-7top8E1WsaiE7OvF3KPnRj08u44Z2_0Rpkty9CDng"));
        let albums: Vec<(&str, u32)> = found.songs.iter().map(|s| (s.album.as_ref().map_or("", |a| a.name.as_str()), s.duration)).collect();
        assert_eq!(albums, [("Robot Rock (Edit)", 188), ("Homework", 329), ("Homework", 317)]);
    }

    #[test]
    fn search_albums_and_artists() {
        let found = search(&json("search_albums"));
        assert!(found.songs.is_empty());
        let a = &found.albums[0];
        assert_eq!((a.id.as_str(), a.title.as_str(), a.artist.as_str(), a.year), ("MPREb_K8qWMWVqXGi", "Random Access Memories", "Daft Punk", 2013));
        assert_eq!((a.artists[0].id.as_str(), a.release_type.as_deref()), (DAFT_PUNK, Some("Album")));
        assert_eq!(found.albums[1].release_type.as_deref(), Some("Single"));
        assert_eq!(found.albums[1].artist, "Skathi");
        let found = search(&json("search_artists"));
        assert_eq!(found.artists.iter().map(|a| (a.id.as_str(), a.name.as_str())).collect::<Vec<_>>(), [(DAFT_PUNK, "Daft Punk"), ("UClYV6hHlupm_S_ObS1W-DYw", "The Weeknd"), ("UCJw8VyO6e3v6S0327AsgwcQ", "Pharrell")]);
        assert!(found.artists[0].thumbnail.is_some());
    }

    /// Album tracks carry no album, thumbnail or year of their own: they take the album's.
    #[test]
    fn album_page() {
        let p = album(&json("album"), "MPREb_K8qWMWVqXGi");
        let a = &p.album;
        assert_eq!((a.title.as_str(), a.artist.as_str(), a.artists[0].id.as_str(), a.year), ("Random Access Memories", "Daft Punk", DAFT_PUNK, 2013));
        assert_eq!(a.release_type.as_deref(), Some("Album"));
        let t: Vec<(u32, &str, u32)> = p.songs.iter().map(|s| (s.track, s.title.as_str(), s.duration)).collect();
        assert_eq!(t, [(1, "Give Life Back to Music", 276), (2, "The Game of Love", 323), (3, "Giorgio by Moroder", 545)]);
        for s in &p.songs {
            assert_eq!((s.album.as_ref().map(|a| (a.id.as_str(), a.name.as_str())), s.year, s.artist.as_str()), (Some(("MPREb_K8qWMWVqXGi", "Random Access Memories")), 2013, "Daft Punk"));
            assert_eq!(s.thumbnail, a.thumbnail);
        }
        assert_eq!(p.songs[0].video_id, "IluRBvnYMoY");
    }

    #[test]
    fn artist_page() {
        let p = artist(&json("artist"), DAFT_PUNK);
        assert_eq!(p.artist.name, "Daft Punk");
        assert_eq!(p.top_songs.len(), 3);
        assert!(p.top_songs.iter().all(|s| !s.video_id.is_empty() && !s.title.is_empty()));
        // Albums and singles, not the playlists it is featured on.
        assert_eq!(p.albums.len(), 6);
        assert!(p.albums.iter().all(|a| a.id.starts_with("MPREb_") && a.artist == "Daft Punk" && a.artists[0].id == DAFT_PUNK && a.year > 1990));
        assert_eq!(p.albums[0].title, "Random Access Memories (Drumless Edition)");
        assert_eq!(p.similar.len(), 3);
        assert!(p.similar.iter().all(|a| a.id.starts_with("UC") && a.id != DAFT_PUNK));
        assert!(p.description.as_deref().is_some_and(|b| b.starts_with("Daft Punk were a French electronic music duo")));
        assert!(p.artist.thumbnail.as_deref().is_some_and(|u| u.starts_with("https://lh3.googleusercontent.com/") && sized_host(u)));
    }

    #[test]
    fn playlist_page() {
        let p = playlist(&json("playlist"), "VLOLAK5uy_lNVBcjNtiCwq-n95uoxJ-Gd4vsfMkyZNs");
        assert_eq!((p.playlist.title.as_str(), p.playlist.song_count), ("Top songs", Some(150)));
        let s = &p.songs[0];
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.duration), ("Instant Crush (feat. Julian Casablancas)", "Daft Punk & Julian Casablancas", 338));
        assert_eq!(s.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Daft Punk", "Julian Casablancas"]);
        assert_eq!(s.album, Some(AlbumLink { id: "MPREb_K8qWMWVqXGi".into(), name: "Random Access Memories".into() }));
        assert_eq!(p.songs[1].artist, "Daft Punk, Pharrell Williams & Nile Rodgers");
        assert_eq!(p.songs.len(), 3);
    }

    #[test]
    fn durations_years_and_thumbnails() {
        assert_eq!(seconds("3:08"), Some(188));
        assert_eq!(seconds("1:02:03"), Some(3723));
        for not in ["98M plays", "3:", ":08", "1:2:3:4", "2013"] {
            assert_eq!(seconds(not), None, "{not}");
        }
        assert_eq!((year("2013"), year("13 songs"), year("20134")), (Some(2013), None, None));
        let t = serde_json::json!({ "thumbnails": [{ "url": "https://i.ytimg.com/vi/x/hq720.jpg" }] });
        assert_eq!(thumbnail(&t).as_deref(), Some("https://i.ytimg.com/vi/x/hq720.jpg"));
        assert_eq!(thumbnail_url("https://i.ytimg.com/vi/x/hq720.jpg", 320), "https://i.ytimg.com/vi/x/hq720.jpg");
        assert_eq!(thumbnail_url("https://yt3.googleusercontent.com/abc", 320), "https://yt3.googleusercontent.com/abc=w320-h320-l90-rj");
    }

    /// The home feed's shelves: songs from list items, playlists and albums from cards, and the
    /// continuation of the page after.
    #[test]
    fn home_feed() {
        let (shelves, next) = feed(&json("home"));
        assert_eq!(next.as_deref(), Some("FIXTURE_CONTINUATION"));
        assert_eq!(shelves.iter().map(|s| (s.title.as_str(), s.items.len())).collect::<Vec<_>>(), [("Quick picks", 3), ("All hits", 2)]);
        let Item::Song(s) = &shelves[0].items[0] else { panic!() };
        assert_eq!((s.video_id.as_str(), s.artist.as_str(), s.artists[0].id.as_str()), ("hHZ4ej144rc", "Vito Bambino", "UC2goE-vgYa9vhMss8WlOm3w"));
        assert_eq!(s.album.as_ref().map(|a| a.id.as_str()), Some("MPREb_belO7bZVmAt"));
        let Item::Playlist(p) = &shelves[1].items[0] else { panic!() };
        assert_eq!((p.id.as_str(), p.title.as_str()), ("VLRDCLAK5uy_nHSqCJjDrW9HBhCNdF6tWPdnOMngOv0wA", "Pop Gold"));
        assert!(p.description.as_deref().is_some_and(|d| d.starts_with("Ed Sheeran, Alicia Keys")));
        assert!(p.thumbnail.as_deref().is_some_and(sized_host));
        let (more, next) = feed(&json("home_more"));
        assert_eq!(next, None);
        let releases = more.iter().find(|s| s.title == "New releases").unwrap();
        assert!(releases.items.iter().all(|i| matches!(i, Item::Album(a) if a.id.starts_with("MPREb_") && !a.title.is_empty())));
    }

    /// Signed-in answers name the account; a visitor's answer says it is no one's.
    #[test]
    fn account_and_visitor_answers() {
        assert_eq!(account_name(&json("account_menu")).as_deref(), Some("Ann Example"));
        assert!(logged_out(&json("signed_out")));
        assert!(!logged_out(&json("library_albums")));
        assert!(!logged_out(&json("album")), "an answer that does not say");
    }

    /// An item without columns or link is dropped; the rest of the page stays.
    #[test]
    fn a_broken_item_drops_alone() {
        let mut v = json("search_songs");
        let shelf = v.pointer_mut("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/1/musicShelfRenderer/contents").unwrap();
        shelf.as_array_mut().unwrap()[0] = serde_json::json!({ "musicResponsiveListItemRenderer": { "flexColumns": [] } });
        assert_eq!(search(&v).songs.iter().map(|s| s.title.as_str()).collect::<Vec<_>>(), ["Da Funk", "Alive"]);
    }
}
