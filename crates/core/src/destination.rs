//! Where the music plays: here, through a plugin's cast, on another of the account's devices this one
//! controls, or in a jam. Every client's player, devices list and media session read this one answer.

use std::sync::Arc;

use crate::remote::Remote;

/// Where the music plays.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Destination {
    /// This device plays, hosting a jam or not.
    Here { hosting: bool },
    /// A plugin's cast from here: `device` takes this device's audio (`sink`), or plays the songs itself.
    Cast { device: String, sink: bool },
    /// Another of the account's devices plays (or one casting for it); this one shows and controls it.
    Mirroring { device: String },
    /// A jam guest: the host's music, listened along to here or only watched.
    Guest,
}

impl Destination {
    /// Whether the system's media controls (Android's media session, MPRIS, Now Playing, the lock screen)
    /// show it: never another device's playback. The system takes what they show for sound coming out of
    /// this device and tells a Bluetooth headset joined to it so: one joined to the other device as well
    /// takes this one for a second source playing, and its pause and play for this one would reach the
    /// other device.
    pub fn shown_to_system(&self) -> bool {
        !matches!(self, Destination::Mirroring { .. })
    }
}

/// [`Destination::shown_to_system`].
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn destination_shown_to_system(destination: Destination) -> bool {
    destination.shown_to_system()
}

/// A plugin's cast from here, as [`destination`] needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Record))]
pub struct Casting {
    pub device: String,
    pub sink: bool,
}

#[cfg(feature = "plugins")]
impl From<&nori_plugins::Cast> for Casting {
    fn from(c: &nori_plugins::Cast) -> Casting {
        Casting { device: c.device.clone(), sink: c.kind == nori_plugins::CastKind::Sink }
    }
}

/// Where the music plays, from the remote control (None: off) and this device's plugin cast.
pub fn destination(remote: Option<Arc<Remote>>, cast: Option<Casting>) -> Destination {
    let (guest, mirrored, hosting) = remote.map_or((false, None, false), |r| r.whereabouts());
    destination_of(guest, mirrored, cast, hosting)
}

/// Where the music plays, from what a client last heard of each part: a jam guest's profile plays its
/// jam; another device mirrored (by its name) outranks a cast, which outranks this device.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn destination_of(guest: bool, mirrored: Option<String>, cast: Option<Casting>, hosting: bool) -> Destination {
    match (guest, mirrored, cast) {
        (true, ..) => Destination::Guest,
        (false, Some(device), _) => Destination::Mirroring { device },
        (false, None, Some(Casting { device, sink })) => Destination::Cast { device, sink },
        (false, None, None) => Destination::Here { hosting },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The system's media controls show this device's playback, a cast from here and a jam guest's jam;
    /// never another device's, even one mirrored while a cast from here runs.
    #[test]
    fn the_system_is_shown_no_other_devices_playback() {
        let speaker = || Some(Casting { device: "Speaker".into(), sink: true });
        let desk = || Some("Desk".to_string());
        for (guest, mirrored, cast, hosting, shown) in [
            (false, None, None, false, true),
            (false, None, None, true, true),
            (false, None, speaker(), false, true),
            (true, desk(), None, false, true),
            (false, desk(), None, false, false),
            (false, desk(), speaker(), true, false),
        ] {
            let d = destination_of(guest, mirrored, cast, hosting);
            assert_eq!(d.shown_to_system(), shown, "{d:?}");
        }
    }
}
