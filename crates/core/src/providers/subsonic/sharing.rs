//! A Navidrome server's public shares, and the playlist calls a jam's share is made of
//! ([`crate::jam_share`]). Each call's answer is data; why sharing failed is a [`ShareError`].

use super::wire;
use crate::client::{pairs, Client, NetResult};
use crate::transport::{self, Exchange, NetError};

/// The first Navidrome that shares unless its admin turned sharing off (major, minor, patch).
const SHARES_BY_DEFAULT: (u32, u32, u32) = (0, 63, 0);

/// Where a public address's own part starts: what follows the server's address.
const SHARE_PATH: &str = "/share/";

/// Why the server would not share.
#[derive(Debug, Clone)]
pub enum ShareError {
    /// Sharing is off on this Navidrome. `off_by_default`: the server is older than 0.63.0, where
    /// sharing is off unless its admin turned it on.
    Off { off_by_default: bool },
    /// The server is not Navidrome, whose public shares a jam is made of.
    NotNavidrome,
    /// The request failed.
    Failed(NetError),
}

impl From<NetError> for ShareError {
    fn from(e: NetError) -> Self {
        ShareError::Failed(e)
    }
}

/// A Navidrome that can share.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Navidrome {
    /// Older than 0.63.0: sharing is off unless its admin turned it on.
    pub off_by_default: bool,
}

impl Navidrome {
    /// A share call's failure: HTTP 501 is Navidrome's answer while sharing is off.
    pub(crate) fn refused(self, e: NetError) -> ShareError {
        match e {
            NetError::Http { status: 501 } => ShareError::Off { off_by_default: self.off_by_default },
            e => ShareError::Failed(e),
        }
    }
}

/// A share: its id, and the path of its page after the server's address (`/share/<id>`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Share {
    pub id: String,
    pub path: String,
}

/// The part of `url` from its `/share/` on: what a guest asks for at the host's own address, since a
/// share names whatever address Navidrome believes it has (an internal one behind octo-fiesta).
fn share_path(url: &str) -> Option<&str> {
    let after_host = url.find("://").map_or(0, |i| i + 3);
    url[after_host..].find(SHARE_PATH).map(|i| &url[after_host + i..])
}

/// `0.64.2 (8a8c0c0)` or `0.63.0-SNAPSHOT` as (major, minor, patch); None when it says no version.
fn version(v: &str) -> Option<(u32, u32, u32)> {
    let mut parts = v.trim().split(|c: char| !c.is_ascii_digit()).map(str::parse::<u32>);
    Some((parts.next()?.ok()?, parts.next()?.ok()?, parts.next()?.ok()?))
}

impl Client {
    /// The server's address its guests are given: the profile's own, not the one Navidrome names.
    pub(crate) fn public_address(&self, path: &str) -> String {
        format!("{}{path}", nori_net::api::normalize(&self.profile.read().url))
    }

    /// Whether the server is a Navidrome, which shares, and since when by default; asked with a ping,
    /// which octo-fiesta passes on to the Navidrome behind it.
    pub(crate) async fn navidrome(&self) -> Result<Navidrome, ShareError> {
        let r = wire::parse(&self.fetch("ping", Vec::new()).await?).map_err(NetError::from)?;
        if !r.kind().eq_ignore_ascii_case("navidrome") {
            return Err(ShareError::NotNavidrome);
        }
        Ok(Navidrome { off_by_default: version(r.server_version()).is_some_and(|v| v < SHARES_BY_DEFAULT) })
    }

    /// One call that changes the server, sent once (never through the offline queue), allowed
    /// `timeout_ms` (0: the platform's default); its answer.
    async fn change(&self, endpoint: &str, params: Vec<(String, String)>, timeout_ms: u32) -> NetResult<Vec<u8>> {
        let body = self.send(endpoint, params, false, timeout_ms).await?;
        wire::parse(&body)?;
        Ok(body)
    }

    /// A new, empty playlist named `name`; its id.
    pub(crate) async fn create_playlist(&self, name: &str) -> NetResult<String> {
        Ok(wire::playlist(&self.change("createPlaylist", pairs(&[("name", name.into())]), 0).await?)?.0)
    }

    /// Adds song `id` to the end of playlist `playlist`. A provider's song (octo-fiesta's `ext-` id) is
    /// fetched into the library first, which can take `timeout_ms`.
    pub(crate) async fn add_to_playlist(&self, playlist: &str, id: &str, timeout_ms: u32) -> NetResult<()> {
        self.change("updatePlaylist", pairs(&[("playlistId", playlist.into()), ("songIdToAdd", id.into())]), timeout_ms).await.map(drop)
    }

    /// Takes the song at `index` out of playlist `playlist`.
    pub(crate) async fn remove_from_playlist(&self, playlist: &str, index: usize) -> NetResult<()> {
        self.change("updatePlaylist", pairs(&[("playlistId", playlist.into()), ("songIndexToRemove", index.to_string())]), 0).await.map(drop)
    }

    pub(crate) async fn delete_playlist(&self, id: &str) -> NetResult<()> {
        self.change("deletePlaylist", pairs(&[("id", id.into())]), 0).await.map(drop)
    }

    /// The library ids of playlist `id`'s songs, in order.
    pub(crate) async fn playlist_song_ids(&self, id: &str) -> NetResult<Vec<String>> {
        let (_, songs) = wire::playlist(&self.fetch("getPlaylist", pairs(&[("id", id.into())])).await?)?;
        Ok(songs.into_iter().map(|s| s.id).collect())
    }

    /// Song `id` as the server has it now: a provider's song octo-fiesta fetched is its library song.
    pub(crate) async fn song_now(&self, id: &str) -> NetResult<crate::Song> {
        Ok(wire::song(&self.fetch("getSong", pairs(&[("id", id.into())])).await?)?)
    }

    /// A public share of playlist `playlist` named `description`, until `expires_ms` (Unix ms).
    pub(crate) async fn create_share(&self, navidrome: Navidrome, playlist: &str, description: &str, expires_ms: i64) -> Result<Share, ShareError> {
        let params = pairs(&[("id", playlist.into()), ("description", description.into()), ("expires", expires_ms.to_string())]);
        let body = self.change("createShare", params, 0).await.map_err(|e| navidrome.refused(e))?;
        let (id, url) = wire::made_share(&body).map_err(NetError::from)?;
        let path = share_path(&url).map_or_else(|| format!("{SHARE_PATH}{id}"), str::to_string);
        Ok(Share { id, path })
    }

    /// Share `share` now lasts until `expires_ms` (Unix ms).
    pub(crate) async fn extend_share(&self, navidrome: Navidrome, share: &Share, description: &str, expires_ms: i64) -> Result<(), ShareError> {
        let params = pairs(&[("id", share.id.clone()), ("description", description.into()), ("expires", expires_ms.to_string())]);
        self.change("updateShare", params, 0).await.map(drop).map_err(|e| navidrome.refused(e))
    }

    pub(crate) async fn delete_share(&self, id: &str) -> NetResult<()> {
        self.change("deleteShare", pairs(&[("id", id.into())]), 0).await.map(drop)
    }

    /// The path of each of `share`'s songs after the server's address (`/share/s/<token>`), in the order
    /// of what it shares, from its public `m3u` list. A song's path holds the share's expiry as it is now.
    pub(crate) async fn shared_paths(&self, share: &Share) -> NetResult<Vec<String>> {
        let body = transport::get(&*self.transport, self.public_address(&format!("{}/m3u", share.path)), 0).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(share_path).map(str::to_string).collect())
    }

    /// The public address of album `id`'s cover; None when it has none.
    pub(crate) async fn album_cover(&self, id: &str) -> NetResult<Option<String>> {
        let cover = wire::album_cover(&self.fetch("getAlbumInfo2", pairs(&[("id", id.into())])).await?)?;
        Ok(cover.as_deref().and_then(share_path).map(|p| self.public_address(p)))
    }

    /// Whether the server answers a request for part of the song at `path` as that part: one byte asked
    /// for, a 206 (or the whole file, from a server that ignores ranges) says yes; octo-fiesta's 200 with
    /// that one byte, labeled as the whole file, says no.
    pub(crate) async fn answers_parts(&self, path: &str) -> NetResult<bool> {
        let headers = [("Range".to_string(), "bytes=0-0".to_string())].into();
        let r = self.transport.send(Exchange { url: self.public_address(path), headers, ..Default::default() }).await?;
        match r.status {
            206 => Ok(true),
            200 => Ok(r.body.len() > 1),
            status => Err(NetError::Http { status }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_share_path_is_what_follows_the_address() {
        for (url, path) in [
            ("http://host.docker.internal:4633/share/s/eyJ.x", Some("/share/s/eyJ.x")),
            ("https://music.example/navidrome/share/abc", Some("/share/abc")),
            ("https://share.example/share/img/tok?size=600", Some("/share/img/tok?size=600")),
            ("https://lastfm.example/i/u/300x300/a.png", None),
        ] {
            assert_eq!(share_path(url), path, "{url}");
        }
    }

    #[test]
    fn versions_read_from_what_navidrome_says() {
        for (said, v) in [("0.64.2 (8a8c0c0)", Some((0, 64, 2))), ("0.63.0-SNAPSHOT", Some((0, 63, 0))), ("0.58.5", Some((0, 58, 5))), ("", None), ("dev", None)] {
            assert_eq!(version(said), v, "{said}");
        }
    }
}
