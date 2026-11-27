//! A scan's plan: what a platform lists now against what the last scan kept. A file at a path kept, of
//! the same size and time, is not read again. A song is the row a scan keeps for its file, not its
//! path: a file that moved or was renamed is known again by its audio's fingerprint ([`rematch`]), so
//! its song, with its plays and seal, stays the same.

use std::collections::HashMap;

use crate::FoundFile;

/// A music file as the last scan kept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    /// The row that is its song.
    pub row: i64,
    pub path: String,
    pub size: u64,
    pub modified_ms: i64,
    /// Its audio's ([`crate::tags::Read::fingerprint`]); None for a file left out as no music nori plays.
    pub fingerprint: Option<u64>,
}

/// What a scan does with each music file.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// Kept rows whose file changed in place: read again.
    pub changed: Vec<(i64, FoundFile)>,
    /// Files at paths not kept: read, then [`rematch`]ed against `gone`.
    pub new: Vec<FoundFile>,
    /// Kept files no longer listed: moved, renamed or deleted.
    pub gone: Vec<Known>,
}

impl Plan {
    /// Nothing to read or drop: the library stands as it is.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.new.is_empty() && self.gone.is_empty()
    }

    /// The files to read.
    pub fn to_read(&self) -> usize {
        self.changed.len() + self.new.len()
    }
}

/// What to do with the music files `found` (other kinds are not planned) when the last scan kept `known`.
pub fn plan(known: Vec<Known>, found: &[FoundFile]) -> Plan {
    let mut kept: HashMap<&str, &Known> = known.iter().map(|k| (k.path.as_str(), k)).collect();
    let mut plan = Plan::default();
    for f in found {
        match kept.remove(f.path.as_str()) {
            Some(k) if k.size == f.size && k.modified_ms == f.modified_ms => {}
            Some(k) => plan.changed.push((k.row, f.clone())),
            None => plan.new.push(f.clone()),
        }
    }
    let mut gone: Vec<Known> = kept.into_values().cloned().collect();
    gone.sort_by(|a, b| a.path.cmp(&b.path));
    plan.gone = gone;
    plan
}

/// The row of a gone file whose audio has `fingerprint`, taken out of `gone`: a new file is that song,
/// moved or renamed. None for a song new to the library.
pub fn rematch(gone: &mut Vec<Known>, fingerprint: u64) -> Option<i64> {
    let at = gone.iter().position(|k| k.fingerprint == Some(fingerprint))?;
    Some(gone.remove(at).row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, size: u64, modified_ms: i64) -> FoundFile {
        FoundFile { path: path.into(), size, modified_ms }
    }

    fn known(row: i64, path: &str, size: u64, fingerprint: u64) -> Known {
        Known { row, path: path.into(), size, modified_ms: 1, fingerprint: Some(fingerprint) }
    }

    #[test]
    fn only_files_new_or_changed_are_read() {
        let kept = vec![known(1, "/m/a.flac", 10, 7), known(2, "/m/b.flac", 10, 8), known(3, "/m/c.flac", 10, 9)];
        let p = plan(kept, &[found("/m/a.flac", 10, 1), found("/m/b.flac", 11, 1), found("/m/d.flac", 10, 1)]);
        assert_eq!(p.changed, vec![(2, found("/m/b.flac", 11, 1))]);
        assert_eq!(p.new, vec![found("/m/d.flac", 10, 1)]);
        assert_eq!(p.gone.iter().map(|k| k.row).collect::<Vec<_>>(), [3]);
        assert_eq!(p.to_read(), 2);
        assert!(plan(vec![known(1, "/m/a.flac", 10, 7)], &[found("/m/a.flac", 10, 1)]).is_empty(), "nothing changed: nothing to do");
    }

    /// A moved file takes the gone row with its fingerprint, once; another new file is a new song.
    #[test]
    fn a_moved_file_is_its_gone_song_again() {
        let mut gone = vec![known(3, "/m/c.flac", 10, 9), known(4, "/m/e.flac", 10, 9)];
        assert_eq!(rematch(&mut gone, 9), Some(3));
        assert_eq!(rematch(&mut gone, 9), Some(4), "a second copy is the second song");
        assert_eq!(rematch(&mut gone, 9), None);
        assert_eq!(rematch(&mut vec![known(5, "/x", 1, 1)], 2), None);
        assert_eq!(rematch(&mut vec![Known { fingerprint: None, ..known(6, "/y", 1, 1) }], 1), None, "a file left out is no song to know again");
    }
}
