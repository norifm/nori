//! A jam's share on the host's Navidrome: guests get the music straight from the server, through one
//! playlist and one public share of it, with no account. The jam's host drives it ([`JamShare`]): each
//! song that joins the queue is added to the playlist, and guests are told its public address and
//! cover address ([`JamShareEvent`]). A provider's song (octo-fiesta's `ext-` id) the host accepted is
//! added the same way, which makes octo-fiesta fetch it into the library first. The share's expiry is
//! extended while the jam runs, and the end deletes the share and the playlist. A jam the app never
//! ended (it died) is cleaned up when the profile next opens ([`Client::clean_jam_leftovers`]).
//!
//! A profile that never hosts a jam pays nothing: no timer, no request, one local read when it opens.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::future::{select, Either};
use nori_tasks::{Notify, Tasks};
use nori_time::Duration;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::client::{Client, NetResult};
use crate::providers::subsonic::{Navidrome, Share};
pub use crate::providers::subsonic::ShareError;
use crate::transport::NetError;
use crate::Song;

/// What a jam's playlist is named with; nori's lists of playlists leave such playlists out.
const JAM_PLAYLIST: &str = "nori-jam-";

/// How long the share lasts from its start or its last extension.
const LASTS: Duration = Duration::from_secs(4 * 3600);
/// How often the share is extended while the jam runs: well before it expires, so an address a guest
/// holds (its token carries the share's expiry as it was) outlives the next one it is told.
const EXTEND_EVERY: Duration = Duration::from_secs(3600);
/// What adding a provider's song may take: octo-fiesta fetches it, then waits up to 120 s for the
/// library scan, which it starts at most once per 30 s.
const PREPARE_MS: u32 = 150_000;
/// Before a provider's song is added again after its scan was missed: octo-fiesta's next scan.
const SCAN_MISSED_WAIT: Duration = Duration::from_secs(35);

/// octo-fiesta's answers to adding a provider's song: it arrived but no scan listed it; the provider
/// failed.
const NOT_FOUND: i32 = 70;
const PROVIDER_FAILED: i32 = 10;

/// Where the profile keeps the share and playlist of the jam it hosts, until they are deleted.
const LEFT: &str = "jam_share";

/// Whether playlist `name` is a jam's ([`JamShare`]), which no list shows.
pub(crate) fn is_jam_playlist(name: &str) -> bool {
    name.starts_with(JAM_PLAYLIST)
}

/// How a jam's guests fetch the shared songs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestFetch {
    /// From any byte: the server answers a request for part of a song with that part.
    FromAnyByte,
    /// Whole, from the start: the server answers a request for part of a song with that part labeled as
    /// the whole file (octo-fiesta in front of Navidrome).
    WholeFromStart,
}

/// Why guests skip a song.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// The provider would not give the song (octo-fiesta's error 10).
    ProviderFailed,
    /// The song arrived but the library did not list it, twice: its scan was missed (error 70).
    NotInLibrary,
    /// The server could not be asked, or refused (the log says why).
    Failed,
}

/// Where a queue entry stands for guests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SharedSong {
    /// A provider's song the server is fetching into its library.
    Preparing,
    /// Guests fetch it at `address`, its cover at `cover`; None when its album has no public cover
    /// address (the jam sends a small picture instead).
    Ready { address: String, cover: Option<String> },
    Skipped { why: Skip },
}

/// What the jam is told of its share.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JamShareEvent {
    /// How guests fetch the share's songs, learned from the first song shared.
    Fetch { how: GuestFetch },
    /// Where the queue entry the jam numbers `entry` stands for guests now. A song's address changes
    /// when the share is extended.
    Song { entry: u64, state: SharedSong },
}

/// Told what becomes of a jam's share, on the thread its tasks run on.
pub trait JamShareHeard: Send + Sync {
    fn heard(&self, event: JamShareEvent);
}

/// The share and playlist of the jam the profile hosts, kept until they are deleted.
#[derive(Serialize, Deserialize)]
struct Left {
    playlist: String,
    share: Option<String>,
}

/// A running jam's share. Dropped without [`JamShare::end`], its share and playlist stay until the
/// profile next opens.
pub struct JamShare {
    shared: Arc<Shared>,
}

struct Shared {
    client: Arc<Client>,
    tasks: Arc<dyn Tasks>,
    heard: Arc<dyn JamShareHeard>,
    navidrome: Navidrome,
    /// The playlist's name, which the share is described with.
    name: String,
    playlist: String,
    share: Share,
    /// The playlist's edits and the reads of what the share holds, one at a time, so a read sees every
    /// edit before it.
    edits: futures_util::lock::Mutex<()>,
    state: Mutex<State>,
    ended: AtomicBool,
    told: Notify,
}

#[derive(Default)]
struct State {
    /// The queue entries shared, by the jam's number for them.
    entries: HashMap<u64, Entry>,
    /// Each album's public cover address, once asked.
    covers: HashMap<String, Option<String>>,
    fetch: Option<GuestFetch>,
}

#[derive(Default)]
struct Entry {
    /// Its library song's id and album, once in the playlist.
    song: Option<(String, Option<String>)>,
    /// What guests were last told of it.
    told: Option<SharedSong>,
}

/// When a share made or extended now expires, Unix ms.
fn expiry() -> i64 {
    crate::db::now_ms() + LASTS.as_millis() as i64
}

/// `r`, with a server's "not found" counted as done: what was to be deleted is gone.
fn gone_ok(r: NetResult<()>) -> NetResult<()> {
    match r {
        Err(NetError::Api { code: NOT_FOUND, .. }) => Ok(()),
        r => r,
    }
}

impl Client {
    /// Starts a jam's share: a playlist named `nori-jam-<short hash>` and one public share of it, which
    /// lasts a few hours and is extended on `tasks`' clock while it runs. `heard` is told what becomes
    /// of each song added.
    pub async fn start_jam_share(self: &Arc<Self>, tasks: Arc<dyn Tasks>, heard: Arc<dyn JamShareHeard>) -> Result<JamShare, ShareError> {
        let navidrome = self.navidrome().await?;
        let name = format!("{JAM_PLAYLIST}{}", &nori_remote::new_id()[..8]);
        let playlist = self.create_playlist(&name).await?;
        self.keep_left(&Left { playlist: playlist.clone(), share: None })?;
        let share = match self.create_share(navidrome, &playlist, &name, expiry()).await {
            Ok(share) => share,
            Err(e) => {
                if self.delete_playlist(&playlist).await.is_ok() {
                    self.forget_left()?;
                }
                return Err(e);
            }
        };
        self.keep_left(&Left { playlist: playlist.clone(), share: Some(share.id.clone()) })?;
        let shared = Arc::new(Shared {
            client: self.clone(),
            tasks: tasks.clone(),
            heard,
            navidrome,
            name,
            playlist,
            share,
            edits: Default::default(),
            state: Default::default(),
            ended: AtomicBool::new(false),
            told: Notify::default(),
        });
        tasks.spawn(Box::pin(shared.clone().extend()));
        Ok(JamShare { shared })
    }

    /// Deletes the share and playlist of a jam this profile hosted and never ended (the app died), when
    /// there is one. Called when the profile opens, before it hosts a jam.
    pub async fn clean_jam_leftovers(&self) -> NetResult<()> {
        let Some(left) = self.left()? else { return Ok(()) };
        if let Some(share) = &left.share {
            gone_ok(self.delete_share(share).await)?;
        }
        gone_ok(self.delete_playlist(&left.playlist).await)?;
        self.forget_left()
    }

    fn left(&self) -> NetResult<Option<Left>> {
        Ok(crate::db::kv_get(&self.core.db.lock(), LEFT)?.and_then(|j| serde_json::from_str(&j).ok()))
    }

    fn keep_left(&self, left: &Left) -> NetResult<()> {
        let json = serde_json::to_string(left).map_err(|e| NetError::Parse { reason: e.to_string() })?;
        Ok(crate::db::kv_put(&self.core.db.lock(), LEFT, &json)?)
    }

    fn forget_left(&self) -> NetResult<()> {
        Ok(crate::db::kv_delete(&self.core.db.lock(), LEFT)?)
    }
}

impl JamShare {
    /// The queue entry the jam numbers `entry` holds `song`: added to the playlist, then told
    /// ([`SharedSong::Ready`]). A provider's song is prepared first ([`SharedSong::Preparing`]): only add
    /// one the host accepted, since adding it makes the server download it.
    pub fn add(&self, entry: u64, song: Song) {
        self.shared.state.lock().entries.insert(entry, Entry::default());
        let shared = self.shared.clone();
        self.shared.tasks.spawn(Box::pin(async move { shared.add(entry, song).await }));
    }

    /// The queue entry `entry` left the queue (played, or taken out): its song leaves the share, and a
    /// guest's request for it is refused from then on (a stream already running finishes).
    pub fn remove(&self, entry: u64) {
        let song = self.shared.state.lock().entries.remove(&entry).and_then(|e| e.song).map(|(id, _)| id);
        if let Some(id) = song {
            let shared = self.shared.clone();
            self.shared.tasks.spawn(Box::pin(async move { shared.take_out(&id).await }));
        }
    }

    /// Ends the share: nothing more is added, told or extended, and the share and the playlist are
    /// deleted. A failure leaves them for [`Client::clean_jam_leftovers`].
    pub async fn end(&self) -> NetResult<()> {
        self.shared.stop();
        let s = &self.shared;
        let _edit = s.edits.lock().await;
        gone_ok(s.client.delete_share(&s.share.id).await)?;
        gone_ok(s.client.delete_playlist(&s.playlist).await)?;
        s.client.forget_left()
    }
}

impl Drop for JamShare {
    fn drop(&mut self) {
        self.shared.stop();
    }
}

impl Shared {
    fn stop(&self) {
        self.ended.store(true, Ordering::Release);
        self.told.notify();
    }

    fn ended(&self) -> bool {
        self.ended.load(Ordering::Acquire)
    }

    /// `f`'s output; None once the share ended first, which drops `f` (a request is called off).
    async fn unless_ended<F: Future>(&self, f: F) -> Option<F::Output> {
        let ended = self.told.until(|| self.ended());
        match select(std::pin::pin!(f), std::pin::pin!(ended)).await {
            Either::Left((out, _)) => Some(out),
            Either::Right(_) => None,
        }
    }

    /// Tells the jam where `entry` stands, when it is still shared and that changed.
    fn tell(&self, entry: u64, state: SharedSong) {
        {
            let mut s = self.state.lock();
            let Some(e) = s.entries.get_mut(&entry) else { return };
            if e.told.as_ref() == Some(&state) {
                return;
            }
            e.told = Some(state.clone());
        }
        self.heard.heard(JamShareEvent::Song { entry, state });
    }

    async fn add(self: Arc<Self>, entry: u64, song: Song) {
        let added = if song.is_provider() {
            self.tell(entry, SharedSong::Preparing);
            self.prepare(&song.id).await
        } else {
            let _edit = self.edits.lock().await;
            if self.ended() {
                return;
            }
            Some(match self.client.add_to_playlist(&self.playlist, &song.id, 0).await {
                Ok(()) => Ok(song),
                Err(e) => {
                    crate::alog::info(&format!("jam share: {} not added: {e}", song.id));
                    Err(Skip::Failed)
                }
            })
        };
        let library = match added {
            None => return,
            Some(Err(why)) => return self.tell(entry, SharedSong::Skipped { why }),
            Some(Ok(library)) => library,
        };
        let shared = match self.state.lock().entries.get_mut(&entry) {
            Some(e) => {
                e.song = Some((library.id.clone(), library.album_id.clone()));
                true
            }
            None => false,
        };
        if shared {
            self.refresh().await;
        } else {
            // Taken out of the queue meanwhile.
            self.take_out(&library.id).await;
        }
    }

    /// Adds provider song `id` to the playlist, which has octo-fiesta fetch it into the library: the
    /// library song it became, or why guests skip it. Its scan missed, it is added once more after
    /// octo-fiesta's next scan. None once the share ended.
    async fn prepare(&self, id: &str) -> Option<Result<Song, Skip>> {
        let mut missed = false;
        loop {
            let why = match self.unless_ended(self.client.add_to_playlist(&self.playlist, id, PREPARE_MS)).await? {
                Ok(()) => break,
                Err(NetError::Api { code: NOT_FOUND, .. }) if !missed => {
                    missed = true;
                    self.unless_ended(self.tasks.sleep(SCAN_MISSED_WAIT)).await?;
                    continue;
                }
                Err(NetError::Api { code: NOT_FOUND, .. }) => Skip::NotInLibrary,
                Err(NetError::Api { code: PROVIDER_FAILED, .. }) => Skip::ProviderFailed,
                Err(e) => {
                    crate::alog::info(&format!("jam share: {id} not prepared: {e}"));
                    Skip::Failed
                }
            };
            return Some(Err(why));
        }
        // octo-fiesta answers a provider's id it fetched with the library song.
        let song = self.unless_ended(self.client.song_now(id)).await?;
        Some(match song {
            Ok(s) if !s.is_provider() => Ok(s),
            Ok(_) => Err(Skip::NotInLibrary),
            Err(e) => {
                crate::alog::info(&format!("jam share: {id} prepared, not found: {e}"));
                Err(Skip::Failed)
            }
        })
    }

    /// Takes one copy of library song `id` out of the playlist.
    async fn take_out(&self, id: &str) {
        let _edit = self.edits.lock().await;
        if self.ended() {
            return;
        }
        let removed = match self.client.playlist_song_ids(&self.playlist).await {
            Ok(ids) => match ids.iter().position(|s| s == id) {
                Some(index) => self.client.remove_from_playlist(&self.playlist, index).await,
                None => Ok(()),
            },
            Err(e) => Err(e),
        };
        if let Err(e) = removed {
            crate::alog::info(&format!("jam share: {id} not taken out: {e}"));
        }
    }

    /// Reads what the share holds now and tells the jam each entry's address and cover; learns how
    /// guests fetch from the first song.
    async fn refresh(&self) {
        let _edit = self.edits.lock().await;
        if self.ended() {
            return;
        }
        // The playlist's songs and the share's addresses are in one order; songs octo-fiesta adds
        // meanwhile only come at the end.
        let read = match self.client.playlist_song_ids(&self.playlist).await {
            Ok(ids) => self.client.shared_paths(&self.share).await.map(|paths| (ids, paths)),
            Err(e) => Err(e),
        };
        let (ids, paths) = match read {
            Ok(read) => read,
            Err(e) => return crate::alog::info(&format!("jam share: not read: {e}")),
        };
        let at: HashMap<&str, &str> = ids.iter().map(String::as_str).zip(paths.iter().map(String::as_str)).collect();
        let unknown = self.state.lock().fetch.is_none();
        if let (true, Some(first)) = (unknown, paths.first()) {
            self.learn_fetch(first).await;
        }
        let shared: Vec<(u64, String, Option<String>)> = {
            let s = self.state.lock();
            s.entries.iter().filter_map(|(&entry, e)| e.song.as_ref().and_then(|(id, album)| Some((entry, at.get(id.as_str())?.to_string(), album.clone())))).collect()
        };
        for (entry, path, album) in shared {
            let cover = match album {
                Some(album) => self.cover(&album).await,
                None => None,
            };
            self.tell(entry, SharedSong::Ready { address: self.client.public_address(&path), cover });
        }
    }

    /// Asks the server for part of the song at `path` and tells the jam how guests must fetch.
    async fn learn_fetch(&self, path: &str) {
        let how = match self.client.answers_parts(path).await {
            Ok(true) => GuestFetch::FromAnyByte,
            Ok(false) => GuestFetch::WholeFromStart,
            Err(e) => return crate::alog::info(&format!("jam share: how guests fetch not learned: {e}")),
        };
        self.state.lock().fetch = Some(how);
        self.heard.heard(JamShareEvent::Fetch { how });
    }

    /// Album `id`'s public cover address, asked once; None when it has none or could not be asked.
    async fn cover(&self, id: &str) -> Option<String> {
        if let Some(cover) = self.state.lock().covers.get(id) {
            return cover.clone();
        }
        match self.client.album_cover(id).await {
            Ok(cover) => {
                self.state.lock().covers.insert(id.to_string(), cover.clone());
                cover
            }
            Err(e) => {
                crate::alog::info(&format!("jam share: album {id}'s cover not asked: {e}"));
                None
            }
        }
    }

    /// Extends the share every [`EXTEND_EVERY`] until it ends, and tells the new addresses.
    async fn extend(self: Arc<Self>) {
        while self.unless_ended(self.tasks.sleep(EXTEND_EVERY)).await.is_some() {
            match self.client.extend_share(self.navidrome, &self.share, &self.name, expiry()).await {
                Ok(()) => self.refresh().await,
                Err(e) => crate::alog::info(&format!("jam share: not extended: {e:?}")),
            }
        }
    }
}

#[cfg(test)]
mod tests;
