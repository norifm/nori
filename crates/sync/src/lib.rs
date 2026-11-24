//! Pairing a person's devices and syncing their profiles through a sync server that reads nothing.
//!
//! A group's devices share one key (keys.rs). A new device joins by showing a code with a one-time
//! lock; a device of the group seals the key to it (pairing.rs). The devices then tell each other their
//! profiles, sealed with the key, through the group's room (session.rs), and merge them (book.rs). The
//! server is a [`Relay`] (room.rs): a real one is a [`Link`] over the platform's WebSockets (link.rs,
//! wire.rs). [`Syncing`] runs a device's life with it (syncing.rs, presence.rs). No I/O here, and nothing
//! at all until a device pairs.

mod book;
mod group;
#[cfg(any(test, feature = "testing"))]
pub mod hub;
mod keys;
mod link;
mod pairing;
mod presence;
mod profile;
mod room;
mod session;
mod syncing;
#[cfg(test)]
mod testing;
mod wire;
mod words;

pub use book::MergeOffer;
pub use group::{paired, ResetNotice, Seen};
pub use keys::RoomName;
pub use link::{Ended, SocketId, Sockets};
pub use pairing::{AdmitStep, Admitting, Came, Confirmation, JoinStep, Joining, PairingRefused, Welcomed, CODE_MS};
pub use presence::Reach;
pub use profile::{Here, NeededPlugin, Plays, SignInCheck, SignInCopy};
pub use room::{Asked, Heard, Member, Refusal, Relay, RoomKind, To};
pub use session::{Outcome, Session, SyncEvent};
pub use syncing::{ActiveProfile, PairingView, ProfilesSynced, SyncClient, SyncState, Syncing};

/// nori's own sync server, used unless the settings name another.
pub const NORI_SERVER: &str = "https://sync.norifm.com";

/// The sync server the settings choose (`sync`, `sync_server`); None when sync is off.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sync_server(on: bool, address: Option<String>) -> Option<String> {
    on.then(|| address.unwrap_or_else(|| NORI_SERVER.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sync_server_is_nori_s_unless_another_is_named_and_none_when_off() {
        assert_eq!(sync_server(true, None).as_deref(), Some(NORI_SERVER));
        assert_eq!(sync_server(true, Some("https://mine.example".into())).as_deref(), Some("https://mine.example"));
        assert_eq!(sync_server(false, Some("https://mine.example".into())), None);
    }
}
