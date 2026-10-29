//! A plugin's cache (`cache`): bytes it can fetch or make again, a file per key in its folder's `cache/`,
//! made at the first put. Within its manifest's quota; a put over it lets the least recently used go
//! first. Kept across updates, removed with the plugin, never backed up.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::runtime::nori::plugin::cache;
use crate::runtime::{types as t, Host};
use crate::session::error;

pub(crate) struct Cache {
    dir: PathBuf,
    quota: u64,
}

/// A key as a file name: hex, so any key is one.
fn file(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

impl Cache {
    pub(crate) fn new(plugin: &Path, quota: u64) -> Cache {
        Cache { dir: plugin.join("cache"), quota }
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(file(key))
    }

    /// Its bytes; the key counts as just used.
    pub(crate) fn get(&self, key: &str) -> Option<Vec<u8>> {
        let path = self.path(key);
        let bytes = fs::read(&path).ok()?;
        let _ = fs::File::options().write(true).open(&path).and_then(|f| f.set_modified(SystemTime::now()));
        Some(bytes)
    }

    pub(crate) fn put(&self, key: &str, value: &[u8]) -> Result<(), t::Error> {
        if value.len() as u64 > self.quota {
            return Err(error(t::ErrorKind::Refused, format!("over its cache's quota of {} bytes", self.quota)));
        }
        let unwritable = |e: std::io::Error| error(t::ErrorKind::Other, e.to_string());
        fs::create_dir_all(&self.dir).map_err(unwritable)?;
        let path = self.path(key);
        fs::write(&path, value).map_err(unwritable)?;
        let mut files: Vec<(SystemTime, u64, PathBuf)> = fs::read_dir(&self.dir)
            .map_err(unwritable)?
            .filter_map(|e| {
                let e = e.ok()?;
                let m = e.metadata().ok()?;
                Some((m.modified().ok()?, m.len(), e.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|f| f.1).sum();
        // Oldest first; the one just put goes last of all.
        files.sort_by_key(|f| (f.2 == path, f.0));
        for (_, len, p) in files {
            if total <= self.quota {
                break;
            }
            if fs::remove_file(&p).is_ok() {
                total -= len;
            }
        }
        Ok(())
    }

    pub(crate) fn remove(&self, key: &str) {
        let _ = fs::remove_file(self.path(key));
    }

    pub(crate) fn file(&self, key: &str) -> Option<String> {
        let path = self.path(key);
        path.is_file().then(|| path.to_string_lossy().into_owned())
    }
}

impl cache::Host for Host {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.cache.get(&key)
    }

    fn put(&mut self, key: String, value: Vec<u8>) -> Result<(), t::Error> {
        self.cache.put(&key, &value)
    }

    fn remove(&mut self, key: String) {
        self.cache.remove(&key);
    }

    fn path(&mut self, key: String) -> Option<String> {
        self.cache.file(&key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_least_recently_used_go_first_and_nothing_is_kept_over_the_quota() {
        let dir = nori_testdir::TempDir::new("plugin-cache");
        let c = Cache::new(dir.path(), 10);
        assert!(!dir.path().join("cache").exists(), "made at the first put");
        c.put("a", b"1234").unwrap();
        // Distinct times, oldest first, whatever the file system's resolution.
        let aged = |key: &str, s: u64| fs::File::options().write(true).open(c.path(key)).unwrap().set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s)).unwrap();
        aged("a", 1);
        c.put("b", b"1234").unwrap();
        aged("b", 2);
        assert_eq!(c.get("a").as_deref(), Some(&b"1234"[..]), "a is used now, b is the least recent");
        c.put("c", b"1234").unwrap();
        assert_eq!((c.get("a").is_some(), c.get("b").is_some(), c.get("c").is_some()), (true, false, true));
        assert_eq!(c.put("big", &[0; 11]).unwrap_err().kind, t::ErrorKind::Refused);
        assert!(c.file("c").is_some_and(|p| p.ends_with("63")) && c.file("b").is_none());
        assert_eq!(Cache::new(dir.path(), 0).put("x", b"1").unwrap_err().kind, t::ErrorKind::Refused, "no quota, no cache");
    }
}
