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

/// What "Resume from server" does with the server's queue.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum ResumePlan {
    /// Nothing was saved: the client says so.
    Nothing,
    /// Play `songs` from `index`, then seek to `position_ms`.
    Play { songs: Vec<Song>, index: u32, position_ms: u64 },
}

pub fn resume_plan(q: PlayQueue) -> ResumePlan {
    if q.songs.is_empty() {
        ResumePlan::Nothing
    } else {
        ResumePlan::Play { songs: q.songs, index: q.index, position_ms: q.position_ms }
    }
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
    fn library_answers() {
        assert_eq!(resume_plan(PlayQueue { songs: vec![], index: 3, position_ms: 9, origin: None }), ResumePlan::Nothing);
        let s = Song { id: "a".into(), ..Default::default() };
        assert_eq!(resume_plan(PlayQueue { songs: vec![s.clone()], index: 0, position_ms: 1200, origin: None }), ResumePlan::Play { songs: vec![s], index: 0, position_ms: 1200 });

        // The local day is near utcs.
        let utc = nori_db::now_ms() / 86_400_000;
        assert!((local_epoch_day() - utc).abs() <= 1);
    }

}
