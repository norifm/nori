//! A local profile's files in a browser: those of the folder the page had picked, handed to this worker
//! as `File`s (the page walks the folder, from its handle or an `<input webkitdirectory>`), read here
//! through `FileReaderSync`, which a Dedicated Worker may block on. Their covers are blob addresses.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom};

use js_sys::{Reflect, Uint8Array};
use nori_core::local::{FileKind, Files, FoundFile, MediaSource, MusicPlace};
use nori_tasks::OneThread;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{Blob, File, FileReaderSync, Url};

/// A profile's folder as the page handed it.
struct Folder {
    profile: String,
    /// Its files by path.
    files: HashMap<String, (FoundFile, File)>,
}

/// The files handed this visit, and the cover addresses made of them.
#[derive(Default)]
struct Handed {
    /// None until the page hands a folder over.
    folder: Option<Folder>,
    /// The blob address of each cover asked for, by its path.
    covers: HashMap<String, String>,
}

impl Handed {
    /// Lets the files and their cover addresses go.
    fn let_go(&mut self) {
        self.folder = None;
        for (_, url) in self.covers.drain() {
            let _ = Url::revoke_object_url(&url);
        }
    }
}

/// The browser's music: the folder picked, as the page handed it. Its place is the device's, as a
/// phone's is: there is nothing to type, the folder is picked.
pub struct WebFiles(OneThread<RefCell<Handed>>);

impl Default for WebFiles {
    fn default() -> Self {
        WebFiles(OneThread(RefCell::default()))
    }
}

impl WebFiles {
    /// The files of profile `id`'s folder, `[{ path, size, modified, file }]` (`modified` in Unix ms), in
    /// place of those handed before; those a scan leaves out are not kept.
    pub fn hand(&self, id: String, list: &js_sys::Array) -> Result<(), JsValue> {
        let mut files = HashMap::new();
        for item in list.iter() {
            let get = |key: &str| Reflect::get(&item, &JsValue::from_str(key));
            let path = get("path")?.as_string().ok_or("a file without its path")?;
            if nori_core::local::kind_of(&path).is_none() {
                continue;
            }
            let size = get("size")?.as_f64().unwrap_or(0.0) as u64;
            let modified_ms = get("modified")?.as_f64().unwrap_or(0.0) as i64;
            let file: File = get("file")?.dyn_into()?;
            files.insert(path.clone(), (FoundFile { path, size, modified_ms }, file));
        }
        let mut handed = self.0.borrow_mut();
        handed.let_go();
        handed.folder = Some(Folder { profile: id, files });
        Ok(())
    }

    /// Profile `id` opens: files handed for another are let go, so they are never read as its own.
    pub fn opened(&self, id: &str) {
        if !self.handed(id) {
            self.0.borrow_mut().let_go();
        }
    }

    /// Whether the files handed are profile `id`'s folder's.
    pub fn handed(&self, id: &str) -> bool {
        self.0.borrow().folder.as_ref().is_some_and(|f| f.profile == id)
    }

    fn file(&self, path: &str) -> Option<File> {
        self.0.borrow().folder.as_ref()?.files.get(path).map(|(_, f)| f.clone())
    }
}

impl Files for WebFiles {
    fn place(&self) -> MusicPlace {
        MusicPlace::Device
    }

    fn list(&self, _folders: &[String]) -> Option<Vec<FoundFile>> {
        Some(self.0.borrow().folder.as_ref()?.files.values().map(|(f, _)| f.clone()).collect())
    }

    fn open(&self, path: &str) -> io::Result<Box<dyn MediaSource>> {
        let file = self.file(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "not among the files handed"))?;
        let reader = FileReaderSync::new().map_err(js_error)?;
        Ok(Box::new(BlobSource { len: file.size() as u64, blob: OneThread(file.into()), reader: OneThread(reader), pos: 0 }))
    }

    /// A picture file's own blob address, or one made of the picture a music file holds (an image is
    /// known by its bytes, whatever its type); empty without one.
    fn cover_address(&self, cover: &str) -> String {
        if let Some(url) = self.0.borrow().covers.get(cover) {
            return url.clone();
        }
        let blob: Option<Blob> = match nori_core::local::kind_of(cover) {
            Some(FileKind::Picture) => self.file(cover).map(Into::into),
            _ => self.open(cover).ok().and_then(|source| nori_core::local::tags::picture(source, &nori_core::local::extension(cover))).and_then(|bytes| Blob::new_with_u8_array_sequence(&js_sys::Array::of1(&Uint8Array::from(bytes.as_slice()))).ok()),
        };
        let Some(url) = blob.and_then(|b| Url::create_object_url_with_blob(&b).ok()) else { return String::new() };
        self.0.borrow_mut().covers.insert(cover.to_string(), url.clone());
        url
    }
}

fn js_error(e: JsValue) -> io::Error {
    io::Error::other(format!("{e:?}"))
}

/// A file read a slice at a time.
struct BlobSource {
    blob: OneThread<Blob>,
    reader: OneThread<FileReaderSync>,
    pos: u64,
    len: u64,
}

impl Read for BlobSource {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let end = (self.pos + buf.len() as u64).min(self.len);
        if end <= self.pos {
            return Ok(0);
        }
        let part = self.blob.slice_with_f64_and_f64(self.pos as f64, end as f64).map_err(js_error)?;
        let bytes = Uint8Array::new(&self.reader.read_as_array_buffer(&part).map_err(js_error)?.into());
        let n = bytes.length() as usize;
        bytes.copy_to(&mut buf[..n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for BlobSource {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let pos = match to {
            SeekFrom::Start(p) => Some(p),
            SeekFrom::End(d) => self.len.checked_add_signed(d),
            SeekFrom::Current(d) => self.pos.checked_add_signed(d),
        };
        self.pos = pos.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "before the start"))?;
        Ok(self.pos)
    }
}

impl MediaSource for BlobSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.len)
    }
}
