//! A plugin's own store: small values by key, one file each in its folder, capped in all.

use std::fs;
use std::path::PathBuf;

/// The most a plugin's store holds, bytes.
pub const STORE_CAP: u64 = 1 << 20;

pub struct Store {
    dir: PathBuf,
}

/// A key as a file name: hex, so any key is one.
fn file(key: &str) -> String {
    key.bytes().map(|b| format!("{b:02x}")).collect()
}

fn key(file: &str) -> Option<String> {
    let bytes: Option<Vec<u8>> = (0..file.len()).step_by(2).map(|i| u8::from_str_radix(file.get(i..i + 2)?, 16).ok()).collect();
    String::from_utf8(bytes?).ok()
}

impl Store {
    pub fn new(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn get(&self, k: &str) -> Option<Vec<u8>> {
        fs::read(self.dir.join(file(k))).ok()
    }

    /// False when it would take the store over [`STORE_CAP`], or can't be written.
    pub fn set(&self, k: &str, value: &[u8]) -> bool {
        let path = self.dir.join(file(k));
        let was = fs::metadata(&path).map_or(0, |m| m.len());
        if self.size() - was + value.len() as u64 > STORE_CAP {
            return false;
        }
        fs::create_dir_all(&self.dir).is_ok() && fs::write(path, value).is_ok()
    }

    pub fn remove(&self, k: &str) {
        let _ = fs::remove_file(self.dir.join(file(k)));
    }

    pub fn keys(&self, prefix: &str) -> Vec<String> {
        let Ok(entries) = fs::read_dir(&self.dir) else { return Vec::new() };
        let mut keys: Vec<String> = entries.filter_map(|e| key(e.ok()?.file_name().to_str()?)).filter(|k| k.starts_with(prefix)).collect();
        keys.sort();
        keys
    }

    fn size(&self) -> u64 {
        fs::read_dir(&self.dir).map_or(0, |entries| entries.filter_map(|e| e.ok()?.metadata().ok()).map(|m| m.len()).sum())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_values_by_key_within_its_cap() {
        let dir = nori_testdir::TempDir::new("plugin-store");
        let s = Store::new(dir.path().join("store"));
        assert_eq!(s.get("a/b"), None);
        assert!(s.set("a/b", b"one"));
        assert!(s.set("a/c", b"two"));
        assert!(s.set("z", b"three"));
        assert_eq!(s.get("a/b").as_deref(), Some(&b"one"[..]));
        assert_eq!(s.keys("a/"), vec!["a/b".to_string(), "a/c".to_string()]);
        s.remove("a/b");
        assert_eq!(s.keys("a/"), vec!["a/c".to_string()]);
        assert!(!s.set("big", &vec![0; STORE_CAP as usize]), "over the cap");
        assert!(s.set("z", &vec![0; (STORE_CAP - 3) as usize]), "a value replaced counts once");
    }
}
