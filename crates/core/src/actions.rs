//! Play-action calls: the songs a radio, instant mix, artist, shuffle or M3U import plays. Taps and
//! shuffle order are nori-queue's.

use crate::autofill::seed_now;
use futures_util::StreamExt;
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult};
use crate::{m3u, Album, Core, Result, Song};

pub use nori_queue::actions::*;

impl Client {
    pub(crate) fn stored_song_of(&self, id: &str) -> Option<Song> {
        self.core.session.song(id).or_else(|| self.core.download_song(id))
    }

    /// Song `id` from the queue store, else the downloads, else the cache or server.
    pub(crate) async fn song_of(&self, id: String) -> NetResult<Song> {
        if let Some(s) = self.stored_song_of(&id) {
            return Ok(s);
        }
        Ok(match self.first(Read::SongById { id: id.clone() }).await? {
            Page::OneSong { v: Some(v) } => v,
            _ => Song::only_id(id),
        })
    }

    /// The library songs of the first `wanted` albums that have any; provider albums and songs are skipped
    /// (playing one makes the server download it).
    pub(crate) async fn library_albums(&self, albums: Vec<Album>, wanted: usize) -> Vec<Song> {
        let mut out = Vec::new();
        let mut found = 0;
        for a in albums.into_iter().filter(|a| !a.is_provider()) {
            let mut songs: Vec<Song> = self.songs(Read::AlbumSongs { id: a.id }).await.unwrap_or_default();
            songs.retain(|s| !s.is_provider());
            if songs.is_empty() {
                continue;
            }
            out.extend(songs);
            found += 1;
            if found == wanted {
                break;
            }
        }
        out
    }
}

impl Core {
    /// The finished download `id`, as stored.
    pub fn download_song(&self, id: &str) -> Option<Song> {
        use rusqlite::OptionalExtension;
        let c = self.db.lock();
        let json: String = c.query_row("SELECT json FROM downloads WHERE server=sid() AND id=?1 AND done=1", [id], |r| r.get(0)).optional().ok()??;
        serde_json::from_str(&json).ok()
    }

    /// Song `id` as the local index keeps it.
    pub(crate) fn indexed_song(&self, id: &str) -> Option<Song> {
        use rusqlite::OptionalExtension;
        let c = self.db.lock();
        let json: String = c.query_row("SELECT json FROM items WHERE server=sid() AND kind=?1 AND id=?2", rusqlite::params![crate::db::ItemKind::Song, id], |r| r.get(0)).optional().ok()??;
        serde_json::from_str(&json).ok()
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Client {
    /// A radio from song `id`: similar songs, else random songs of its genre ([radio_queue]).
    pub async fn radio(&self, id: String) -> NetResult<Vec<Song>> {
        let similar = self.songs(Read::SimilarSongs { id: id.clone(), count: RADIO }).await?;
        let listed = similar.iter().find(|s| s.id == id);
        let seed = match (self.stored_song_of(&id), listed) {
            // A YouTube Home row says no length; the radio's copy does.
            (Some(seed), Some(l)) if seed.duration == 0 => Song { duration: l.duration, ..seed },
            (Some(seed), _) => seed,
            (None, Some(l)) => l.clone(),
            (None, None) => self.song_of(id).await?,
        };
        if let Some(queue) = radio_queue(seed.clone(), similar) {
            return Ok(queue);
        }
        let random = self.songs(Read::RandomSongs { size: RADIO, genre: seed.genre.clone() }).await?;
        Ok(radio_queue(seed.clone(), random).unwrap_or_else(|| vec![seed]))
    }

    /// A mix around song `id` from the index and history; the radio when that yields only the song.
    pub async fn instant_mix(&self, id: String) -> NetResult<Vec<Song>> {
        let mix = self.core.mix_instant(id.clone(), INSTANT_MIX, seed_now())?;
        if mix.len() > 1 {
            return Ok(mix);
        }
        self.radio(id).await
    }

    /// The library songs of `albums` in order, skipping provider and unreadable albums.
    pub async fn artist_songs(&self, albums: Vec<Album>) -> Vec<Song> {
        let reads = albums.into_iter().filter(|a| !a.is_provider()).map(|a| self.cached_or_fetched(Read::AlbumById { id: a.id }));
        // A few at a time: an artist can have dozens of albums.
        let pages: Vec<_> = futures_util::stream::iter(reads).buffered(4).collect().await;
        let mut songs: Vec<Song> = pages.into_iter().filter_map(|p| p.ok().flatten()).flat_map(Page::songs).collect();
        songs.retain(|s| !s.is_provider());
        songs
    }

    /// What playing artist page `page` plays: its albums' songs ([`Client::artist_songs`]), or the page's
    /// own songs when it lists no albums (a YouTube Music profile's videos).
    pub async fn artist_page_songs(&self, page: crate::ArtistDetail) -> Vec<Song> {
        if page.albums.is_empty() {
            return page.songs;
        }
        self.artist_songs(page.albums).await
    }

    /// Random library songs.
    pub async fn shuffle_all(&self) -> NetResult<Vec<Song>> {
        let mut songs = self.songs(Read::RandomSongs { size: SHUFFLE_ALL, genre: None }).await?;
        songs.retain(|s| !s.is_provider());
        Ok(songs)
    }

    /// One random library album, whole; refills add more ([`crate::OriginKind::ShuffleAlbums`]).
    pub async fn shuffle_albums(&self) -> NetResult<Vec<Song>> {
        let albums = self.read_now(Read::AlbumList { kind: crate::browse::AlbumSort::Random, size: SHUFFLE_ALBUMS, offset: 0, genre: None }).await?.albums();
        Ok(self.library_albums(albums, 1).await)
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Matches an M3U against the index; unmatched entries are only counted.
    pub fn m3u_import(&self, text: String) -> Result<M3uImport> {
        let matched = self.m3u_match(m3u::m3u_parse(text))?;
        let song_ids: Vec<String> = matched.iter().flatten().map(|s| s.id.clone()).collect();
        Ok(M3uImport { entries: matched.len() as u32, song_ids })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::client::tests::{block, client};
    use crate::client::NetProfile;
    use crate::{db, history::tests::song};

    fn songs_json(key: &str, ids: &[&str]) -> String {
        let s: Vec<String> = ids.iter().map(|i| format!(r#"{{"id":"{i}","title":"{i}","isDir":false}}"#)).collect();
        format!(r#"{{"subsonic-response":{{"status":"ok","{key}":{{"song":[{}]}}}}}}"#, s.join(","))
    }

    #[test]
    fn radio_falls_back_to_genre() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let seed = Song { id: "r".into(), genre: Some("Jazz".into()), ..Default::default() };
        c.core.session.register(vec![seed.clone()]);
        fake.answer(&songs_json("similarSongs2", &["r"]));
        fake.answer(&songs_json("randomSongs", &["x", "r", "ext-deezer-song-1", "y"]));
        let got = block(c.radio(seed.id.clone())).unwrap();
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["r", "x", "y"]);
        let asked = fake.asked();
        assert!(asked[1].contains("getRandomSongs") && asked[1].contains("genre=Jazz"), "{asked:?}");
        // Nothing indexed near the song: the instant mix is the radio.
        fake.answer(&songs_json("similarSongs2", &["s"]));
        assert_eq!(block(c.instant_mix(seed.id)).unwrap().len(), 2);
    }

    #[test]
    fn song_of_unknown_id_reads_server() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","song":{"id":"not-queued","title":"Q","genre":"Rock","isDir":false}}}"#);
        let s = block(c.song_of("not-queued".into())).unwrap();
        assert_eq!((s.title.as_str(), s.genre.as_deref()), ("Q", Some("Rock")));
        assert!(fake.asked()[0].contains("getSong"), "{:?}", fake.asked());
    }

    #[test]
    fn artist_albums_cached_while_fresh() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let album = |id: &str| Album { id: id.into(), ..Default::default() };
        let page = |songs: &str| format!(r#"{{"subsonic-response":{{"status":"ok","album":{{"id":"a","name":"a","song":[{songs}]}}}}}}"#);
        let one = r#"{"id":"1","title":"1","isDir":false}"#;
        let two = r#"{"id":"2","title":"2","isDir":false}"#;
        let play = || block(c.artist_songs(vec![album("a")])).len();
        fake.answer(&page(one));
        assert_eq!((play(), play(), fake.asked().len()), (1, 1, 1));
        // Aged: read again, so a track added since plays; offline, the aged page plays.
        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        fake.answer(&page(&format!("{one},{two}")));
        assert_eq!(play(), 2);
        c.core.db.lock().execute("UPDATE cache SET ts = 0", []).unwrap();
        fake.fail(crate::transport::FailureKind::Connect);
        assert_eq!(play(), 2);
    }

    #[test]
    fn artist_albums_read_four_at_a_time_in_order() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let albums: Vec<Album> = (0..10).map(|i| Album { id: format!("a{i}"), ..Default::default() }).collect();
        for i in 0..10 {
            fake.answer(&format!(r#"{{"subsonic-response":{{"status":"ok","album":{{"id":"a{i}","name":"a{i}","song":[{{"id":"{i}","title":"{i}","isDir":false}}]}}}}}}"#));
        }
        *fake.pends.lock() = true;
        let mut play = std::pin::pin!(c.artist_songs(albums));
        let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
        assert!(std::future::Future::poll(play.as_mut(), &mut cx).is_pending());
        assert!(fake.asked().len() <= 4, "{} asked at once", fake.asked().len());
        let got = block(play);
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"]);
    }

    /// An artist without albums plays the songs its page shows; one with albums plays those.
    #[test]
    fn an_artist_without_albums_plays_its_songs() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let song = |id: &str| Song { id: id.into(), ..Default::default() };
        let bare = crate::ArtistDetail::new(Default::default(), Vec::new(), vec![song("v1"), song("v2")]);
        assert_eq!(block(c.artist_page_songs(bare)).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["v1", "v2"]);
        fake.answer(r#"{"subsonic-response":{"status":"ok","album":{"id":"a","name":"a","song":[{"id":"1","title":"1","isDir":false}]}}}"#);
        let with_album = crate::ArtistDetail::new(Default::default(), vec![Album { id: "a".into(), ..Default::default() }], vec![song("v1")]);
        assert_eq!(block(c.artist_page_songs(with_album)).iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["1"]);
    }

    #[test]
    fn providers_skipped() {
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        let album = |id: &str, is_external| Album { id: id.into(), is_external, ..Default::default() };
        fake.answer(r#"{"subsonic-response":{"status":"ok","album":{"id":"a1","name":"a1","song":[{"id":"1","title":"1","isDir":false},{"id":"ext-deezer-song-2","title":"2","isDir":false}]}}}"#);
        let got = block(c.artist_songs(vec![album("a1", false), album("ext-2", true), album("pl-deezer-3", false)]));
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["1"]);
        assert_eq!(fake.asked().len(), 1);
        fake.answer(&songs_json("randomSongs", &["x", "ext-deezer-song-1"]));
        assert_eq!(block(c.shuffle_all()).unwrap().iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["x"]);

        // Shuffle albums plays one library album.
        let (c, fake) = client(NetProfile { url: "h".into(), ..Default::default() });
        fake.answer(r#"{"subsonic-response":{"status":"ok","albumList2":{"album":[{"id":"b","name":"B"},{"id":"ext-applemusic-album-1","name":"X","isExternal":true},{"id":"a","name":"A"}]}}}"#);
        let album = |id: &str, songs: &[&str]| {
            let s: Vec<String> = songs.iter().map(|i| format!(r#"{{"id":"{i}","title":"{i}","isDir":false}}"#)).collect();
            format!(r#"{{"subsonic-response":{{"status":"ok","album":{{"id":"{id}","name":"{id}","song":[{}]}}}}}}"#, s.join(","))
        };
        fake.answer(&album("b", &["b1", "ext-deezer-song-9", "b2", "b3"]));
        fake.answer(&album("a", &["a1", "a2"]));
        let got = block(c.shuffle_albums()).unwrap();
        assert_eq!(got.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b1", "b2", "b3"]);
        let asked = fake.asked();
        assert!(asked[0].contains("getAlbumList2") && asked[0].contains("type=random"), "{asked:?}");
        assert_eq!(asked.len(), 2, "{asked:?}");
    }

    #[test]
    fn m3u_import_counts_matches() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        db::index(&mut core.db.lock(), &[], &[], &[song("1", "Dogs", "Pink Floyd", "Animals", "", 1977)]).unwrap();
        let text = "#EXTM3U\n#EXTINF:200,Pink Floyd - Dogs\na.flac\n#EXTINF:100,Nobody - Nothing\nb.flac\n";
        let r = core.m3u_import(text.into()).unwrap();
        assert_eq!((r.song_ids.as_slice(), r.entries), (["1".to_string()].as_slice(), 2));
        let none = core.m3u_import("#EXTM3U\n#EXTINF:1,X - Y\nc.mp3\n".into()).unwrap();
        assert!(none.song_ids.is_empty());
        assert_eq!(none.entries, 1);
    }
}
