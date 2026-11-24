//! A device's profile sync: which of its profiles is which synced profile, the order of their changes,
//! the removal markers, and the merge with what the group's other devices say.

use nori_settings::settings::{new_server_id, SavedServer};
use serde::{Deserialize, Serialize};

use crate::keys::{b64, random};
use crate::profile::{body, guest, sign_in_check, Here, NeededPlugin, Plays, Shared, SignInCheck, Stamp};

/// One synced profile and this device's profile it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Record {
    /// As this device last sent or took it.
    shared: Shared,
    /// This device's own id for the profile, which its downloads, history and queue are kept under.
    local: String,
    /// Added on this device by sync, not by the user here.
    from_sync: bool,
    /// Sync this profile is off here: the device keeps it, and what the others say about it is ignored.
    off: bool,
}

/// A profile removed everywhere; it stays, so the profile does not come back from another device. A
/// merge's marker names the profile it became.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Marker {
    pub id: String,
    pub stamp: Stamp,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub into: Option<String>,
}

/// Profiles and markers, as one device tells another.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Changes {
    #[serde(default)]
    pub profiles: Vec<Shared>,
    #[serde(default)]
    pub removed: Vec<Marker>,
}

impl Changes {
    pub(crate) fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.removed.is_empty()
    }

    /// As told to a device that plays `plays`: without a sign-in it could not use.
    pub(crate) fn fitted(self, plays: Plays) -> Changes {
        Changes { profiles: self.profiles.into_iter().map(|p| p.fitted(plays)).collect(), removed: self.removed }
    }
}

/// Two profiles here that nori cannot tell are one (a home address on one device, a public one on the
/// other): both show, and the user may merge them.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct MergeOffer {
    /// This device's ids of the two profiles.
    pub a: String,
    pub b: String,
}

/// What taking the others' profiles did here.
#[derive(Debug, Default, PartialEq)]
pub(crate) struct Taken {
    /// This device's profiles changed.
    pub changed: bool,
    /// Plugins the profiles need that are not installed here.
    pub install: Vec<NeededPlugin>,
}

/// This device's profile sync.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Book {
    records: Vec<Record>,
    removed: Vec<Marker>,
    /// The highest counter seen.
    clock: u64,
    /// Pairs of profiles (sync ids) the user chose to keep apart.
    #[serde(default)]
    apart: Vec<[String; 2]>,
}

/// An address as two devices may write it.
fn address(url: &str) -> String {
    url.trim().trim_end_matches('/').to_ascii_lowercase()
}

/// Whether `a` and `b` are one profile: the same kind, server address and user, or the same account.
fn same(a: &SavedServer, a_account: Option<&str>, b: &SavedServer, b_account: Option<&str>) -> bool {
    let at_server = !a.url.trim().is_empty() && address(&a.url) == address(&b.url) && a.user == b.user;
    a.kind == b.kind && (at_server || (a_account.is_some() && a_account == b_account))
}

/// Whether `a` and `b` may be one profile nori cannot tell apart: the same kind and user at two
/// addresses, one of them reached only at home or named as the other's second address.
fn maybe_same(a: &SavedServer, b: &SavedServer) -> bool {
    let (ua, ub) = (address(&a.url), address(&b.url));
    let home = nori_remote::is_home_only(&a.url) != nori_remote::is_home_only(&b.url);
    let second = address(&a.alt_url) == ub || address(&b.alt_url) == ua;
    a.kind == b.kind && !a.user.is_empty() && a.user == b.user && !ua.is_empty() && !ub.is_empty() && ua != ub && (home || second)
}

/// A new sync identity.
fn new_sync_id() -> String {
    b64(&random::<16>())
}

impl Book {
    /// A stamp past every one seen.
    pub(crate) fn next(&mut self, by: &str, now_ms: i64) -> Stamp {
        self.clock += 1;
        Stamp { n: self.clock, ms: now_ms, by: by.to_string() }
    }

    pub(crate) fn seen(&mut self, stamp: &Stamp) {
        self.clock = self.clock.max(stamp.n);
    }

    fn mark(&mut self, marker: Marker) {
        self.removed.retain(|m| m.id != marker.id);
        self.removed.push(marker);
    }

    /// Takes this device's changes to `servers` into the book, each with a new stamp: a profile added,
    /// edited, removed or switched back on. A profile switched off here without the question is kept
    /// on the other devices.
    pub(crate) fn notice(&mut self, servers: &[SavedServer], here: &Here, by: &str, now_ms: i64) -> Changes {
        let mut out = Changes::default();
        for s in servers.iter().filter(|s| !guest(s)) {
            match self.records.iter().position(|r| r.local == s.id) {
                Some(i) if s.sync_off => self.records[i].off = true,
                None if s.sync_off => {}
                Some(i) => {
                    let before = &self.records[i].shared;
                    let profile = body(s, here, Some(&before.profile));
                    let plugin = here.plugin(&s.kind).or_else(|| before.plugin.clone());
                    if self.records[i].off || profile != before.profile || plugin != before.plugin {
                        let stamp = self.next(by, now_ms);
                        let r = &mut self.records[i];
                        (r.off, r.shared.profile, r.shared.plugin, r.shared.stamp) = (false, profile, plugin, stamp);
                        let id = r.shared.id.clone();
                        self.removed.retain(|m| m.id != id);
                        out.profiles.push(self.records[i].shared.clone());
                    }
                }
                None => {
                    let shared = Shared { id: new_sync_id(), stamp: self.next(by, now_ms), profile: body(s, here, None), account: None, plugin: here.plugin(&s.kind), newer: Default::default() };
                    out.profiles.push(shared.clone());
                    self.records.push(Record { shared, local: s.id.clone(), from_sync: false, off: false });
                }
            }
        }
        let (kept, gone): (Vec<Record>, Vec<Record>) = std::mem::take(&mut self.records).into_iter().partition(|r| servers.iter().any(|s| s.id == r.local));
        self.records = kept;
        for r in gone.into_iter().filter(|r| !r.off) {
            let marker = Marker { id: r.shared.id, stamp: self.next(by, now_ms), into: None };
            self.mark(marker.clone());
            out.removed.push(marker);
        }
        out
    }

    /// Everything this device holds, as a device that knows nothing would need it.
    pub(crate) fn state(&self) -> Changes {
        let profiles = self.records.iter().filter(|r| !r.off && r.shared.stamp != Stamp::default()).map(|r| r.shared.clone()).collect();
        Changes { profiles, removed: self.removed.clone() }
    }

    /// What this device holds newer than `theirs`, and when `theirs` is a whole state, what it lacks.
    pub(crate) fn answer(&self, theirs: &Changes, whole: bool) -> Changes {
        let mine = self.state();
        let stamp_of = |id: &str| theirs.profiles.iter().find(|p| p.id == id).map(|p| &p.stamp).or_else(|| theirs.removed.iter().find(|m| m.id == id).map(|m| &m.stamp));
        let newer = |id: &str, stamp: &Stamp| stamp_of(id).map_or(whole, |theirs| stamp > theirs);
        Changes {
            profiles: mine.profiles.into_iter().filter(|p| newer(&p.id, &p.stamp)).collect(),
            removed: mine.removed.into_iter().filter(|m| newer(&m.id, &m.stamp)).collect(),
        }
    }

    /// Takes what another device said into `servers`, this device's profiles.
    pub(crate) fn take(&mut self, theirs: Changes, servers: &mut Vec<SavedServer>, here: &Here) -> Taken {
        let mut taken = Taken::default();
        for m in &theirs.removed {
            taken.changed |= self.fold(m, servers);
        }
        for p in theirs.profiles {
            if let (Some(plugin), true) = (&p.plugin, here.plays.plays(&p.kind())) {
                if here.plugin(&p.kind()).is_none() && !taken.install.contains(plugin) {
                    taken.install.push(plugin.clone());
                }
            }
            taken.changed |= self.take_one(p, servers, here);
        }
        taken
    }

    /// Applies marker `m`; true when this device's profiles changed.
    fn fold(&mut self, m: &Marker, servers: &mut Vec<SavedServer>) -> bool {
        self.seen(&m.stamp);
        if self.removed.iter().any(|k| k.id == m.id && k.stamp >= m.stamp) {
            return false;
        }
        self.mark(m.clone());
        let Some(at) = self.records.iter().position(|r| r.shared.id == m.id && !r.off) else { return false };
        if self.records[at].shared.stamp > m.stamp {
            return false;
        }
        let gone = self.records.remove(at);
        let Some(into) = &m.into else {
            servers.retain(|s| s.id != gone.local);
            return true;
        };
        match self.records.iter().position(|r| &r.shared.id == into && !r.off) {
            // Both are here: one goes, and the profile the user added on this device stays as the merged
            // one, with its downloads and history.
            Some(t) if self.records[t].from_sync && !gone.from_sync => {
                let kept = &mut self.records[t];
                let dropped = std::mem::replace(&mut kept.local, gone.local);
                kept.from_sync = false;
                let made = servers.iter().find(|s| s.id == kept.local).and_then(|s| kept.shared.server(Some(s), &kept.local));
                servers.retain(|s| s.id != dropped);
                if let (Some(made), Some(s)) = (made, servers.iter_mut().find(|s| s.id == kept.local)) {
                    *s = made;
                }
            }
            Some(_) => servers.retain(|s| s.id != gone.local),
            // The merged one has not arrived: this profile waits as it, behind every change to it.
            None => self.records.push(Record { shared: Shared { id: into.clone(), stamp: Stamp::default(), ..gone.shared }, ..gone }),
        }
        true
    }

    /// Takes profile `p`; true when this device's profiles changed.
    fn take_one(&mut self, p: Shared, servers: &mut Vec<SavedServer>, here: &Here) -> bool {
        self.seen(&p.stamp);
        if let Some(m) = self.removed.iter().position(|m| m.id == p.id) {
            if self.removed[m].stamp >= p.stamp {
                return false;
            }
            self.removed.remove(m);
        }
        if let Some(i) = self.records.iter().position(|r| r.shared.id == p.id) {
            return !self.records[i].off && self.records[i].shared.stamp < p.stamp && self.apply(i, p, servers, here);
        }
        let Some(theirs) = p.server(None, "") else { return false };
        let local = |r: &Record| servers.iter().find(|s| s.id == r.local);
        if let Some(i) = self.records.iter().position(|r| local(r).is_some_and(|s| same(s, r.shared.account.as_deref(), &theirs, p.account.as_deref()))) {
            // One profile under two sync identities: both devices keep the lower.
            if self.records[i].off {
                return false;
            }
            if p.id < self.records[i].shared.id {
                self.records[i].shared.id = p.id.clone();
            }
            return self.records[i].shared.stamp < p.stamp && self.apply(i, p, servers, here);
        }
        if servers.iter().any(|s| s.sync_off && same(s, None, &theirs, p.account.as_deref())) {
            return false;
        }
        let id = new_server_id();
        let Some(made) = p.server(None, &id) else { return false };
        let shared = Shared { profile: body(&made, here, Some(&p.profile)), ..p };
        servers.push(made);
        self.records.push(Record { shared, local: id, from_sync: true, off: false });
        true
    }

    /// Lays `p` over record `i`'s profile.
    fn apply(&mut self, i: usize, p: Shared, servers: &mut Vec<SavedServer>, here: &Here) -> bool {
        let r = &mut self.records[i];
        let at = servers.iter().position(|s| s.id == r.local);
        let Some(made) = p.server(at.map(|i| &servers[i]), &r.local) else { return false };
        let plugin = p.plugin.clone().or_else(|| r.shared.plugin.clone());
        r.shared = Shared { profile: body(&made, here, Some(&p.profile)), plugin, id: r.shared.id.clone(), ..p };
        match at {
            Some(at) => servers[at] = made,
            None => servers.push(made),
        }
        true
    }

    fn record(&self, local: &str) -> Option<&Record> {
        self.records.iter().find(|r| r.local == local && !r.off)
    }

    /// Whether profile `local` syncs: removing it or turning its sync off reaches the other devices.
    pub(crate) fn synced(&self, local: &str) -> bool {
        self.record(local).is_some()
    }

    /// Sync this profile off for `local`; with `elsewhere`, it is removed from the other devices too.
    pub(crate) fn sync_off(&mut self, local: &str, elsewhere: bool, by: &str, now_ms: i64) -> Changes {
        let Some(i) = self.records.iter().position(|r| r.local == local && !r.off) else { return Changes::default() };
        self.records[i].off = true;
        if !elsewhere {
            return Changes::default();
        }
        let marker = Marker { id: self.records[i].shared.id.clone(), stamp: self.next(by, now_ms), into: None };
        self.mark(marker.clone());
        Changes { profiles: Vec::new(), removed: vec![marker] }
    }

    /// The profiles here that may be one; none the user kept apart.
    pub(crate) fn offers(&self, servers: &[SavedServer]) -> Vec<MergeOffer> {
        let on: Vec<(&Record, &SavedServer)> = self.records.iter().filter(|r| !r.off).filter_map(|r| Some((r, servers.iter().find(|s| s.id == r.local)?))).collect();
        let apart = |a: &Record, b: &Record| self.apart.iter().any(|p| p.contains(&a.shared.id) && p.contains(&b.shared.id));
        let mut offers = Vec::new();
        for (i, (ra, a)) in on.iter().enumerate() {
            for (rb, b) in &on[i + 1..] {
                if (ra.from_sync || rb.from_sync) && maybe_same(a, b) && !apart(ra, rb) {
                    offers.push(MergeOffer { a: a.id.clone(), b: b.id.clone() });
                }
            }
        }
        offers
    }

    /// Merges profile `drop` into `keep` on every device: `keep`'s settings win, and each device keeps
    /// the profile its user added there.
    pub(crate) fn merge(&mut self, keep: &str, drop: &str, servers: &mut Vec<SavedServer>, by: &str, now_ms: i64) -> Changes {
        let (Some(k), Some(d)) = (self.record(keep).cloned(), self.record(drop).cloned()) else { return Changes::default() };
        let mut out = Changes::default();
        let stamp = self.next(by, now_ms);
        if let Some(r) = self.records.iter_mut().find(|r| r.shared.id == k.shared.id) {
            r.shared.stamp = stamp;
            out.profiles.push(r.shared.clone());
        }
        let marker = Marker { id: d.shared.id, stamp: self.next(by, now_ms), into: Some(k.shared.id) };
        self.fold(&marker, servers);
        out.removed.push(marker);
        out
    }

    /// Keeps profiles `a` and `b` apart: they are offered for merging no more.
    pub(crate) fn keep_apart(&mut self, a: &str, b: &str) {
        if let (Some(a), Some(b)) = (self.record(a), self.record(b)) {
            let pair = [a.shared.id.clone(), b.shared.id.clone()];
            self.apart.push(pair);
        }
    }

    /// Checks a sign-in on this device to `account` against the one `local` syncs with; the first known
    /// account becomes the profile's.
    pub(crate) fn signed_in(&mut self, local: &str, account: Option<String>, by: &str, now_ms: i64) -> (SignInCheck, Changes) {
        let Some(i) = self.records.iter().position(|r| r.local == local && !r.off) else { return (SignInCheck::Accepted, Changes::default()) };
        let check = sign_in_check(self.records[i].shared.account.as_deref(), account.as_deref());
        if check != SignInCheck::Accepted || self.records[i].shared.account.is_some() {
            return (check, Changes::default());
        }
        let stamp = self.next(by, now_ms);
        let r = &mut self.records[i];
        (r.shared.account, r.shared.stamp) = (account, stamp);
        (check, Changes { profiles: vec![r.shared.clone()], removed: Vec::new() })
    }

    /// This device's profiles whose sign-in the group's devices hold.
    pub(crate) fn signed_in_elsewhere(&self) -> Vec<String> {
        self.records.iter().filter(|r| !r.off && ["password", "apiKey"].iter().any(|k| r.shared.profile.get(*k).and_then(|v| v.as_str()).is_some_and(|v| !v.is_empty()))).map(|r| r.local.clone()).collect()
    }
}

#[cfg(test)]
mod tests;
