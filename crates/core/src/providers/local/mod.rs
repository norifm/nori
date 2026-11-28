//! Music files on this device as a profile: the platform lists them ([`Files`]), a scan reads what
//! changed into the offline index (scan.rs), and every read answers from that index (reads.rs). There is
//! no account and nothing is streamed: a song plays from its file ([`Provider::file`]). Its favorites
//! are the index's, so they follow a song that moved; its playlists are the `.m3u` files beside the
//! music, read only.

mod reads;
mod scan;

use std::io;
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

pub use nori_local::{DevicePlaylist, Files, FoundFile, MediaSource, MusicPlace};

use super::{nowhere, Answer, FieldSpec, FormField, Provider, ProviderInfo, Stream};
use crate::browse::{LibrarySection, ProfileRules, SettingsPart};
use crate::cache_policy::Read;
use crate::client::{Client, NetResult, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::StreamQuality;
use crate::transport::Transport;
use crate::{Core, IngestStats};

/// A device's media library as its platform lists it, behind the platform's permission to read it:
/// Android's MediaStore. Rust reads the files at the paths it lists.
#[cfg_attr(feature = "ffi", uniffi::export(with_foreign))]
pub trait DeviceMusic: Send + Sync {
    /// Every music file the library lists, with its size and time; None while it may not be read.
    fn list(&self) -> Option<Vec<FoundFile>>;

    /// The album picture the library has for the music file at `path` (its own, or its folder's, which
    /// the library reads where the app may not), kept as a picture file the app reads; None without one.
    fn album_art(&self, path: String) -> Option<String>;

    /// The playlists the library lists (the `.m3u` files in its folders), their songs by path.
    fn playlists(&self) -> Vec<DevicePlaylist>;
}

/// The device's music a [`DeviceMusic`] lists, opened from disk.
pub(crate) struct Listed(pub(crate) Arc<dyn DeviceMusic>);

impl Files for Listed {
    fn place(&self) -> MusicPlace {
        MusicPlace::Device
    }

    fn list(&self, _folders: &[String]) -> Option<Vec<FoundFile>> {
        self.0.list()
    }

    fn album_art(&self, music: &str) -> Option<String> {
        self.0.album_art(music.to_string())
    }

    fn playlists(&self) -> Vec<DevicePlaylist> {
        self.0.playlists()
    }
}

/// A song's file on this device, which it plays from: read through its platform's [`Files`].
#[derive(Clone)]
pub struct SongFile {
    pub path: String,
    files: Arc<dyn Files>,
}

impl SongFile {
    pub fn open(&self) -> io::Result<Box<dyn MediaSource>> {
        self.files.open(&self.path)
    }

    /// Its extension, which says what it holds.
    pub fn extension(&self) -> String {
        nori_local::extension(&self.path)
    }
}

/// How far a library's sync is: the files read of those it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct SyncProgress {
    pub read: u32,
    pub of: u32,
}

/// The rules of a local profile.
fn rules() -> ProfileRules {
    use LibrarySection::*;
    use SettingsPart::*;
    ProfileRules {
        asks: false,
        account: false,
        playlists: false,
        downloads: false,
        radio: false,
        indexed: true,
        rescans: true,
        sections: vec![Albums, Favourites, Artists, Songs, Playlists, Smart, History, Genres, Decades, Folders],
        settings: vec![Profiles, ServerOptions, Look, Playback, Sound, Lyrics, Library, Storage, About],
        remote: true,
        measures: true,
        stars: true,
        stars_here: false,
        scrobbles: false,
        feed: false,
        mixes: true,
    }
}

/// A local profile open.
pub(crate) struct Local {
    files: Arc<dyn Files>,
    /// The open profile's folders, where the user picks them.
    folders: RwLock<Vec<String>>,
    /// The sync running, if one is; a second waits for it.
    progress: Mutex<Option<SyncProgress>>,
    syncing: futures_util::lock::Mutex<()>,
}

impl Local {
    pub(crate) fn new(files: Arc<dyn Files>) -> Local {
        Local { files, folders: RwLock::default(), progress: Mutex::default(), syncing: futures_util::lock::Mutex::new(()) }
    }
}

#[async_trait::async_trait]
impl Provider for Local {
    fn info(&self) -> ProviderInfo {
        let field = |what, advanced| FieldSpec { what, advanced };
        let fields = match self.files.place() {
            MusicPlace::Folders => vec![field(FormField::Name, false), field(FormField::Folders, false)],
            MusicPlace::Device => vec![field(FormField::Name, false)],
        };
        ProviderInfo { kind: ServerKind::Local, fields, sign_in: Vec::new(), identities: None }
    }

    /// Nothing to fill: a profile that names no folders reads the platform's own.
    fn ready(&self, _: &SavedServer) -> bool {
        true
    }

    async fn check(&self, _: Arc<dyn Transport>, profile: SavedServer) -> NetResult<SavedServer> {
        let folders = match (self.files.place(), profile.folders.is_empty()) {
            (MusicPlace::Folders, true) => self.files.default_folders(),
            _ => profile.folders.clone(),
        };
        Ok(SavedServer { folders, ..profile })
    }

    fn rules(&self, _: &SavedServer, _: bool) -> ProfileRules {
        rules()
    }

    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()> {
        core.serves(ServerKind::Local.id())?;
        *core.server.write() = crate::api::Server::default();
        *core.account.write() = None;
        *self.folders.write() = profile.folders.clone();
        core.set_rules(rules());
        Ok(())
    }

    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>> {
        let folders = self.folders.read().clone();
        Ok(reads::page(&client.core, read, &folders)?.map(|p| Answer::Page(Box::new(p))))
    }

    async fn write(&self, client: &Client, w: Write) -> NetResult<()> {
        match w {
            Write::Star { kind, id, on } => Ok(reads::star(&client.core, kind, &id, on)?),
            _ => Err(nowhere("write")),
        }
    }

    /// Its file; a song plays from it ([`Provider::file`]), never through a stream.
    fn stream(&self, client: &Client, id: String, _: StreamQuality) -> Stream {
        let url = reads::path(&client.core, &id).unwrap_or_default();
        Stream { url, quality: "0".into() }
    }

    /// The next song opens from its file: nothing is fetched ahead.
    fn ahead(&self) -> Option<usize> {
        Some(0)
    }

    fn cover_address(&self, _: &Core, cover: &str, _: u32) -> String {
        self.files.cover_address(cover)
    }

    fn file(&self, client: &Client, id: &str) -> Option<SongFile> {
        Some(SongFile { path: reads::path(&client.core, id)?, files: self.files.clone() })
    }

    async fn sync(&self, client: &Client) -> NetResult<IngestStats> {
        let _one = self.syncing.lock().await;
        let folders = self.folders.read().clone();
        let total = scan::scan(&client.core, &*self.files, &folders, |p| *self.progress.lock() = p).await;
        *self.progress.lock() = None;
        Ok(total?)
    }

    fn sync_progress(&self) -> Option<SyncProgress> {
        *self.progress.lock()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nori_local::testing::flac;
    use nori_testdir::TempDir;

    use super::*;
    use crate::browse::AlbumSort;
    use crate::cache_policy::Page;
    use crate::client::tests::{block, Fake};
    use crate::client::Starrable;
    use crate::providers::Kinds;

    /// The folder's files, music opened counted.
    struct Counted(nori_local::Disk, AtomicUsize);

    impl Files for Counted {
        fn place(&self) -> MusicPlace {
            self.0.place()
        }

        fn list(&self, folders: &[String]) -> Option<Vec<FoundFile>> {
            self.0.list(folders)
        }

        fn open(&self, path: &str) -> std::io::Result<Box<dyn nori_local::MediaSource>> {
            if nori_local::kind_of(path) == Some(nori_local::FileKind::Music) {
                self.1.fetch_add(1, Ordering::Relaxed);
            }
            self.0.open(path)
        }
    }

    fn put(dir: &Path, rel: &str, bytes: &[u8]) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }

    /// Two albums, one a compilation, a broken file, a cover and a playlist.
    fn library(dir: &Path) {
        let ann = |title: &str, n: &str| vec![format!("TITLE={title}"), "ARTIST=Ann".into(), "ALBUMARTIST=Ann".into(), "ALBUM=First".into(), format!("TRACKNUMBER={n}"), "GENRE=Jazz".into(), "DATE=1997".into()];
        fn tags(t: &[String]) -> Vec<&str> {
            t.iter().map(String::as_str).collect()
        }
        put(dir, "Ann/First/02.flac", &flac(550.0, &tags(&ann("Closing", "2")), None));
        put(dir, "Ann/First/01.flac", &flac(440.0, &tags(&ann("Opening", "1")), None));
        put(dir, "Ann/First/cover.jpg", b"jpeg");
        put(dir, "Hits/01.flac", &flac(660.0, &["TITLE=Up", "ARTIST=Cy", "ALBUM=Hits"], Some(b"png")));
        put(dir, "Hits/02.flac", &flac(770.0, &["TITLE=Down", "ARTIST=Di", "ALBUM=Hits"], None));
        put(dir, "Hits/broken.mp3", &b"no music here".repeat(64));
        put(dir, "lists/mix.m3u", b"#EXTM3U\n../Hits/02.flac\n../Ann/First/01.flac\n");
    }

    /// A client over a local profile reading `dir` through `files`.
    fn opened(dir: &Path, files: Arc<dyn Files>) -> Arc<Client> {
        let core = Core::new(String::new(), "l".into(), Arc::default()).unwrap();
        let kinds = Kinds::builtin();
        kinds.set_files(files);
        core.use_kinds(kinds);
        core.configure_profile(SavedServer { id: "l".into(), kind: ServerKind::Local, folders: vec![dir.to_string_lossy().into()], ..Default::default() }).unwrap();
        Client::new(core, Arc::new(Fake::default()), Default::default())
    }

    fn read(c: &Client, r: Read) -> Page {
        block(c.read_now(r)).unwrap()
    }

    fn titles(songs: &[crate::Song]) -> Vec<&str> {
        songs.iter().map(|s| s.title.as_str()).collect()
    }

    fn song(c: &Client, title: &str) -> crate::Song {
        let Page::Found { v } = read(c, Read::Search { query: title.into(), songs: 5, albums: 0, artists: 0 }) else { panic!() };
        v.songs.into_iter().find(|s| s.title == title).unwrap_or_else(|| panic!("{title} is found"))
    }

    #[test]
    fn a_folder_reads_as_a_library() {
        let dir = TempDir::new("local-library");
        library(&dir);
        let c = opened(&dir, Arc::new(nori_local::Disk::folders()));
        let n = block(c.sync_library()).unwrap();
        assert_eq!((n.artists, n.albums, n.songs), (3, 2, 4), "the broken file is left out");
        assert_eq!(c.sync_progress(), None);
        let Page::Albums { v } = read(&c, Read::AlbumList { kind: AlbumSort::ByName, size: 10, offset: 0, genre: None }) else { panic!() };
        assert_eq!(v.iter().map(|a| (a.name.as_str(), a.artist.as_str(), a.is_compilation)).collect::<Vec<_>>(), [("First", "Ann", false), ("Hits", "Cy, Di", true)]);
        let Page::AlbumPage { v: first } = read(&c, Read::AlbumById { id: v[0].id.clone() }) else { panic!() };
        assert_eq!(titles(&first.songs), ["Opening", "Closing"]);
        let cover = dir.join("Ann/First/cover.jpg").to_string_lossy().into_owned();
        assert_eq!(first.album.cover_art.as_deref(), Some(cover.as_str()));
        assert_eq!(c.core.cover_address(cover.clone(), 300).as_deref(), Some(format!("file://{cover}").as_str()));
        assert_eq!(nori_local::cover_bytes(&cover).unwrap(), b"jpeg");
        let up = song(&c, "Up");
        assert_eq!(nori_local::cover_bytes(up.cover_art.as_deref().unwrap()).unwrap(), b"png", "the picture the file holds");
        assert_eq!(c.file_of(&crate::ItemKey::own(&up.id)).map(|f| f.path), Some(dir.join("Hits/01.flac").to_string_lossy().into_owned()));

        let Page::Artists { v: artists } = read(&c, Read::ArtistIndex) else { panic!() };
        assert_eq!(artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Ann", "Cy", "Di"]);
        let Page::ArtistPage { v: cy } = read(&c, Read::ArtistById { id: artists[1].id.clone() }) else { panic!() };
        assert!(cy.albums.is_empty() && titles(&cy.songs) == ["Up"], "a compilation's artist shows its songs");
        let Page::Genres { v } = read(&c, Read::GenreList) else { panic!() };
        assert_eq!(v.iter().map(|g| (g.name.as_str(), g.song_count, g.album_count)).collect::<Vec<_>>(), [("Jazz", 2, 1)]);

        let Page::Playlists { v } = read(&c, Read::PlaylistList) else { panic!() };
        assert_eq!(v.iter().map(|p| (p.name.as_str(), p.song_count)).collect::<Vec<_>>(), [("mix", 2)]);
        let Page::PlaylistPage { v } = read(&c, Read::PlaylistById { id: v[0].id.clone() }) else { panic!() };
        assert_eq!(titles(&v.songs), ["Down", "Opening"]);

        let Page::Artists { v: roots } = read(&c, Read::FolderIndex) else { panic!() };
        assert_eq!(roots.len(), 1);
        let Page::DirectoryPage { v: top } = read(&c, Read::FolderById { id: roots[0].id.clone() }) else { panic!() };
        assert_eq!((top.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), top.songs.len()), (vec!["Ann", "Hits"], 0), "folders without music are left out");
        let Page::DirectoryPage { v: hits } = read(&c, Read::FolderById { id: top.folders[1].id.clone() }) else { panic!() };
        assert_eq!(titles(&hits.songs), ["Up", "Down"]);
    }

    /// A file moved to another folder, renamed and tagged anew is the same song: its plays and seal stay.
    #[test]
    fn a_moved_renamed_file_keeps_its_plays_and_seal() {
        let dir = TempDir::new("local-moved");
        library(&dir);
        let c = opened(&dir, Arc::new(nori_local::Disk::folders()));
        block(c.sync_library()).unwrap();
        let opening = song(&c, "Opening");
        block(c.write(Write::Star { kind: Starrable::Song, id: opening.id.clone(), on: true })).unwrap();
        // The test's files last a second; a listen is a song's whole length.
        crate::history::tests::listen(&c.core, &crate::Song { duration: 200, ..opening.clone() }, crate::history::tests::NOW - 1000);
        std::fs::remove_file(dir.join("Ann/First/01.flac")).unwrap();
        put(&dir, "Moved/opening (remaster).flac", &flac(440.0, &["TITLE=Opening (Remaster)", "ARTIST=Ann", "ALBUM=First"], None));
        std::fs::remove_file(dir.join("Hits/02.flac")).unwrap();
        block(c.sync_library()).unwrap();
        let moved = song(&c, "Opening (Remaster)");
        assert_eq!((moved.id.as_str(), moved.starred), (opening.id.as_str(), true));
        let Page::StarredPage { v } = read(&c, Read::StarredItems) else { panic!() };
        assert_eq!(titles(&v.songs), ["Opening (Remaster)"]);
        let (plays, _) = c.core.history_recent(10, None, false).unwrap();
        assert_eq!(plays.iter().map(|p| p.song.id.as_str()).collect::<Vec<_>>(), [opening.id.as_str()]);
        let Page::Found { v } = read(&c, Read::Search { query: "down".into(), songs: 5, albums: 5, artists: 5 }) else { panic!() };
        assert!(v.songs.is_empty(), "a deleted file's song is gone");
    }

    /// A folder that can't be read (a drive unplugged, a permission taken back) leaves the library as it
    /// was, seals and all; read again, it is the same library.
    #[test]
    fn an_unreadable_folder_keeps_the_library() {
        let dir = TempDir::new("local-unreadable");
        let music = dir.join("music");
        library(&music);
        let c = opened(&music, Arc::new(nori_local::Disk::folders()));
        block(c.sync_library()).unwrap();
        let up = song(&c, "Up");
        block(c.write(Write::Star { kind: Starrable::Song, id: up.id.clone(), on: true })).unwrap();
        std::fs::rename(&music, dir.join("away")).unwrap();
        let n = block(c.sync_library()).unwrap();
        assert_eq!(n.songs, 4, "nothing dropped");
        std::fs::rename(dir.join("away"), &music).unwrap();
        block(c.sync_library()).unwrap();
        let back = song(&c, "Up");
        assert_eq!((back.id, back.starred), (up.id, true));
    }

    /// A sync with nothing changed reads no file; a file changed in place is the only one read again.
    #[test]
    fn a_rescan_reads_only_what_changed() {
        let dir = TempDir::new("local-rescan");
        library(&dir);
        let files = Arc::new(Counted(nori_local::Disk::folders(), AtomicUsize::new(0)));
        let c = opened(&dir, files.clone());
        block(c.sync_library()).unwrap();
        assert_eq!(files.1.swap(0, Ordering::Relaxed), 5, "every music file once");
        block(c.sync_library()).unwrap();
        assert_eq!(files.1.load(Ordering::Relaxed), 0);
        put(&dir, "Hits/01.flac", &flac(660.0, &["TITLE=Up Again", "ARTIST=Cy", "ALBUM=Hits"], None));
        block(c.sync_library()).unwrap();
        assert_eq!(files.1.load(Ordering::Relaxed), 1);
        assert_eq!(song(&c, "Up Again").cover_art, None, "it holds no picture now");
    }

    /// A phone's media library over a folder: it lists the music only, finds a folder's picture for an
    /// album whose files hold none, and keeps a playlist.
    struct Phone(std::path::PathBuf);

    impl Files for Phone {
        fn place(&self) -> MusicPlace {
            MusicPlace::Device
        }

        fn list(&self, _: &[String]) -> Option<Vec<FoundFile>> {
            let all = nori_local::Disk::device(self.0.to_string_lossy().into()).list(&[])?;
            Some(all.into_iter().filter(|f| nori_local::kind_of(&f.path) == Some(nori_local::FileKind::Music)).collect())
        }

        fn album_art(&self, music: &str) -> Option<String> {
            let art = Path::new(music).with_file_name("cover.jpg");
            art.exists().then(|| art.to_string_lossy().into_owned())
        }

        fn playlists(&self) -> Vec<DevicePlaylist> {
            let at = |rel: &str| self.0.join(rel).to_string_lossy().into_owned();
            vec![DevicePlaylist { path: at("road.m3u"), name: "Road".into(), songs: vec![at("Hits/02.flac"), at("gone.flac"), at("Ann/First/02.flac")] }]
        }
    }

    /// Where the platform lists only the music, an album without a picture of its own takes the one the
    /// platform finds for it, and its playlists are the platform's.
    #[test]
    fn a_phone_gives_covers_and_playlists_it_lists() {
        let dir = TempDir::new("local-phone");
        library(&dir);
        let c = opened(&dir, Arc::new(Phone(dir.to_path_buf())));
        block(c.sync_library()).unwrap();
        let Page::Albums { v } = read(&c, Read::AlbumList { kind: AlbumSort::ByName, size: 10, offset: 0, genre: None }) else { panic!() };
        let cover = dir.join("Ann/First/cover.jpg").to_string_lossy().into_owned();
        assert_eq!(v[0].cover_art.as_deref(), Some(cover.as_str()), "the platform's picture");
        assert_eq!(v[1].cover_art.as_deref(), Some(dir.join("Hits/01.flac").to_string_lossy().as_ref()), "the file's own");
        assert_eq!(song(&c, "Opening").cover_art.as_deref(), Some(cover.as_str()));
        let Page::Playlists { v } = read(&c, Read::PlaylistList) else { panic!() };
        assert_eq!(v.iter().map(|p| (p.name.as_str(), p.song_count)).collect::<Vec<_>>(), [("Road", 2)], "a file it doesn't have is left out");
        let Page::PlaylistPage { v } = read(&c, Read::PlaylistById { id: v[0].id.clone() }) else { panic!() };
        assert_eq!(titles(&v.songs), ["Down", "Closing"]);
    }

    /// A computer's files, its Music folder `/m/Music`.
    struct Computer;

    impl Files for Computer {
        fn place(&self) -> MusicPlace {
            MusicPlace::Folders
        }

        fn list(&self, _: &[String]) -> Option<Vec<FoundFile>> {
            Some(Vec::new())
        }

        fn default_folders(&self) -> Vec<String> {
            vec!["/m/Music".into()]
        }
    }

    /// A local profile is offered only where the device's files are given, and is added with nothing to
    /// fill: one that names no folders reads the platform's own, and only its saved form shows them.
    #[test]
    fn a_local_profile_is_added_with_nothing_to_fill() {
        use crate::providers::{form_fields, FirstStep, Platform};
        let kinds = Kinds::builtin();
        assert!(!kinds.providers().iter().any(|p| p.kind == ServerKind::Local));
        let new = SavedServer { kind: ServerKind::Local, ..Default::default() };
        assert!(!kinds.server_ready(new.clone()));
        for (files, saved_form) in [(Arc::new(Computer) as Arc<dyn Files>, vec![FormField::Folders]), (Arc::new(nori_local::Disk::device("/m".into())), vec![])] {
            let kinds = Kinds::builtin();
            kinds.set_files(files);
            let info = kinds.providers().into_iter().find(|p| p.kind == ServerKind::Local).unwrap();
            assert_eq!(info.first_step(), FirstStep::AddNow);
            assert!(kinds.server_ready(new.clone()));
            let shown = |adding| form_fields(&info, Platform::Computer, &new, adding).iter().map(|f| f.what).collect::<Vec<_>>();
            assert_eq!((shown(true), shown(false)), (vec![FormField::Name], saved_form));
        }
        let local = Local::new(Arc::new(Computer));
        let checked = |folders: Vec<String>| block(local.check(Arc::new(Fake::default()), SavedServer { folders, ..new.clone() })).unwrap().folders;
        assert_eq!(checked(Vec::new()), ["/m/Music"], "the Music folder");
        assert_eq!(checked(vec!["/rips".into()]), ["/rips"], "the folders it names");
    }
}
