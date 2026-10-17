//! Small decisions every client shares: the local day, the random-songs fallback of a mix, and what
//! resuming the server's queue plays.

use nori_model::{Album, PlayQueue, Song};

/// Finished downloads, grouped by album in newest-download order, with unfiled songs kept separately.
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct DownloadedLibrary {
    pub albums: Vec<Album>,
    pub songs: Vec<Song>,
}

struct DownloadedAlbum {
    album: Album,
    songs: Vec<Song>,
}

impl DownloadedAlbum {
    fn detail(mut self) -> crate::pages::AlbumDetail {
        self.songs.sort_by_key(|s| (s.disc_number, s.track));
        let mut detail = crate::pages::AlbumDetail::new(self.album, self.songs, Vec::new());
        detail.queue = crate::pages::PageQueue::of(nori_model::OriginKind::Downloads, &detail.album.id);
        detail
    }
}

pub fn downloaded_library(songs: Vec<Song>) -> DownloadedLibrary {
    let (albums, songs) = group_downloads(songs);
    DownloadedLibrary { albums: albums.into_iter().map(|g| g.album).collect(), songs }
}

pub fn downloaded_album(songs: Vec<Song>) -> Option<crate::pages::AlbumDetail> {
    group_downloads(songs).0.into_iter().next().map(|g| g.detail())
}

fn group_downloads(songs: Vec<Song>) -> (Vec<DownloadedAlbum>, Vec<Song>) {
    let mut groups: Vec<DownloadedAlbum> = Vec::new();
    let mut loose = Vec::new();
    let mut albums = std::collections::HashMap::<String, usize>::new();
    for song in songs {
        let Some(id) = song.album_id.as_ref().filter(|id| !id.is_empty()) else {
            loose.push(song);
            continue;
        };
        let at = *albums.entry(id.clone()).or_insert_with(|| {
            groups.push(DownloadedAlbum {
                album: Album {
                    id: id.clone(), name: song.album.clone(), artist: song.artist.clone(),
                    artist_id: song.artist_id.clone(), cover_art: song.cover_art.clone(), year: song.year,
                    subtitle: nori_model::lines::album_subtitle(&song.artist, song.year),
                    ..Default::default()
                },
                songs: Vec::new(),
            });
            groups.len() - 1
        });
        let group = &mut groups[at];
        group.album.song_count += 1;
        group.album.duration = group.album.duration.saturating_add(song.duration);
        if group.album.cover_art.is_none() {
            group.album.cover_art = song.cover_art.clone();
        }
        group.songs.push(song);
    }
    (groups, loose)
}

/// How many random songs stand in for a mix the index could not draw.
pub const MIX_FALLBACK_SONGS: i32 = 50;

/// Today in local time, as days since 1970: mixes are drawn per local day.
pub fn local_epoch_day() -> i64 {
    let now = nori_db::now_ms() / 1000;
    (now + local_offset_s(now)).div_euclid(86_400)
}

/// The local UTC offset at `at_s` (seconds since 1970), in seconds; 0 where the C library cannot say.
#[cfg_attr(not(unix), allow(unused_variables))]
pub fn local_offset_s(at_s: i64) -> i64 {
    #[cfg(unix)]
    unsafe {
        let t = at_s as libc::time_t;
        let mut tm: libc::tm = std::mem::zeroed();
        if !libc::localtime_r(&t, &mut tm).is_null() {
            return tm.tm_gmtoff as i64;
        }
    }
    0
}

/// The lyrics of nothing playing.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn lyrics_none() -> nori_model::Lyrics {
    nori_model::Lyrics::default()
}

/// Who left a queue on the server: nori on another device, or another app, by the name it gave.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum QueueBy {
    Nori,
    App { name: String },
}

/// The queue another device left on the server, offered here: `songs` from `index`, `position_ms` in.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct QueueOffer {
    pub by: QueueBy,
    pub songs: Vec<Song>,
    pub index: u32,
    pub position_ms: u64,
}

impl QueueOffer {
    /// Where the offered queue stands.
    pub fn place(&self) -> Option<QueuePlace> {
        place(&self.songs, self.index, self.position_ms)
    }
}

/// A song in a queue and how far into it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuePlace {
    pub song: String,
    pub position_ms: u64,
}

fn place(songs: &[Song], index: u32, position_ms: u64) -> Option<QueuePlace> {
    songs.get(index as usize).map(|s| QueuePlace { song: s.id.clone(), position_ms })
}

/// The queue `saved` on the server by the app named `by`, offered to a device showing its song `here`
/// (or nothing) unless the offer does not stand ([`offer_stands`]), it stands where this device left it
/// (`ours`), or it was offered before (`seen`).
pub fn queue_offer(saved: PlayQueue, by: &str, here: Option<&str>, playing: bool, ours: Option<&QueuePlace>, seen: Option<&QueuePlace>) -> Option<QueueOffer> {
    let at = place(&saved.songs, saved.index, saved.position_ms)?;
    if !offer_stands(&at.song, here.map(String::from), playing) || ours == Some(&at) || seen == Some(&at) {
        return None;
    }
    let by = if by == nori_net::api::CLIENT { QueueBy::Nori } else { QueueBy::App { name: by.to_string() } };
    Some(QueueOffer { by, songs: saved.songs, index: saved.index, position_ms: saved.position_ms })
}

/// Whether an offer of a queue at the song `offered` is still worth showing on a device that shows the song
/// `here` (its own, or another device's it controls) and is `playing` or not.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn offer_stands(offered: &str, here: Option<String>, playing: bool) -> bool {
    !playing && here.as_deref() != Some(offered)
}

/// The favourites handed to the mixes from the stored answer, and what `mix_favourites_refresh` needs.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct FavouritesHanded {
    pub handed: bool,
    pub digest: Option<u64>,
    /// Young enough that the server is not asked.
    pub fresh: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downloads_group_partial_albums_and_keep_unfiled_songs() {
        let song = |id: &str, album: Option<&str>, disc, track| Song {
            id: id.into(), album_id: album.map(Into::into), album: "Shared title".into(),
            disc_number: disc, track, duration: 60, ..Default::default()
        };
        let mut first = song("b2", Some("b"), 2, 2);
        first.cover_art = Some("cover-b".into());
        let (mut groups, loose) = group_downloads(vec![
            first, song("a2", Some("a"), 1, 2), song("loose", None, 0, 0),
            song("b1", Some("b"), 1, 1), song("empty", Some(""), 0, 0),
            song("b3", Some("b"), 2, 1),
        ]);
        assert_eq!(groups.iter().map(|g| g.album.id.as_str()).collect::<Vec<_>>(), ["b", "a"]);
        assert_eq!((groups[0].album.song_count, groups[0].album.duration), (3, 180));
        assert_eq!(groups[0].album.cover_art.as_deref(), Some("cover-b"));
        assert_eq!(groups[1].songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["a2"]);
        assert_eq!(loose.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["loose", "empty"]);
        let detail = groups.remove(0).detail();
        assert_eq!(detail.songs.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b1", "b3", "b2"]);
        assert_eq!(detail.queue.origin().kind, nori_model::OriginKind::Downloads);
    }

    #[test]
    fn a_queue_left_elsewhere_is_offered_once() {
        let songs: Vec<Song> = ["a", "b"].iter().map(|id| Song { id: id.to_string(), ..Default::default() }).collect();
        let saved = PlayQueue { songs: songs.clone(), index: 1, position_ms: 1200, origin: None };
        let at = QueuePlace { song: "b".into(), position_ms: 1200 };
        let elsewhere = QueuePlace { song: "b".into(), position_ms: 900 };
        let offer = |by: &str, here, playing, ours, seen| queue_offer(saved.clone(), by, here, playing, ours, seen);
        let web = offer("NavidromeUI", None, false, None, None).unwrap();
        assert_eq!((web.by.clone(), web.index, web.position_ms), (QueueBy::App { name: "NavidromeUI".into() }, 1, 1200));
        assert_eq!(web.place(), Some(at.clone()));
        assert_eq!(offer("nori", Some("a"), false, Some(&elsewhere), None).map(|o| o.by), Some(QueueBy::Nori), "another nori, further on");
        for (why, here, playing, ours, seen) in [
            ("something plays", None, true, None, None),
            ("its song is here", Some("b"), false, None, None),
            ("this device left it", None, false, Some(&at), None),
            ("offered before", None, false, None, Some(&at)),
        ] {
            assert_eq!(offer("nori", here, playing, ours, seen), None, "{why}");
        }
        assert_eq!(queue_offer(PlayQueue { songs: vec![], index: 0, position_ms: 0, origin: None }, "x", None, false, None, None), None, "nothing saved");
    }

    #[test]
    fn library_answers() {
        // The local day is near utcs.
        let utc = nori_db::now_ms() / 86_400_000;
        assert!((local_epoch_day() - utc).abs() <= 1);
    }

}
