//! How a jam's guest gets a plugin provider's songs: what the plugin declares ([`GuestStreams`]), and
//! where a guest gets each song from ([`GuestSource`]).

/// How a jam's guest gets a plugin provider's songs, as the plugin's manifest declares. A plugin that
/// says nothing, or one this device doesn't have, is [`GuestStreams::SameNetwork`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestStreams {
    /// Only through the host's device, on its network.
    SameNetwork,
    /// The address the host resolves works on any device for hours, though the service could stop
    /// that without warning (YouTube Music's names the device that asked).
    HostAddress,
    /// The address works only for the device that asked, or only for minutes (SoundCloud's); the
    /// song's id works anywhere, signed out.
    GuestLookup,
}

/// Where a jam's guest gets a song from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSource {
    /// The host's device answers the guest's reads and streams, as a LAN jam's guest plays.
    Host,
    /// The address the host resolved and sent. With `then_lookup`, when that address fails, the
    /// guest's own plugin looks the song up by its id.
    HostAddress { then_lookup: bool },
    /// The guest's own plugin looks the song up by its id, signed out.
    Lookup,
    /// Nowhere: the guest sees the queue and asks for songs.
    RequestsOnly,
}

impl GuestStreams {
    /// Where a guest gets a song: `same_network` when it reaches the host's device directly,
    /// `plugin_runs` when the provider's plugin runs on the guest's device.
    pub fn source(self, same_network: bool, plugin_runs: bool) -> GuestSource {
        match self {
            _ if same_network => GuestSource::Host,
            GuestStreams::HostAddress => GuestSource::HostAddress { then_lookup: plugin_runs },
            GuestStreams::GuestLookup if plugin_runs => GuestSource::Lookup,
            GuestStreams::GuestLookup | GuestStreams::SameNetwork => GuestSource::RequestsOnly,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_guest_gets_a_song_where_the_plugin_says_it_works() {
        use GuestSource as S;
        use GuestStreams as D;
        // Declared, same network, the plugin runs on the guest: where the song comes from.
        let table = [
            (D::SameNetwork, true, false, S::Host),
            (D::SameNetwork, false, true, S::RequestsOnly),
            (D::HostAddress, true, true, S::Host),
            (D::HostAddress, false, true, S::HostAddress { then_lookup: true }),
            (D::HostAddress, false, false, S::HostAddress { then_lookup: false }),
            (D::GuestLookup, true, false, S::Host),
            (D::GuestLookup, false, true, S::Lookup),
            (D::GuestLookup, false, false, S::RequestsOnly),
        ];
        for (declared, same_network, plugin_runs, source) in table {
            assert_eq!(declared.source(same_network, plugin_runs), source, "{declared:?}, same network {same_network}, plugin runs {plugin_runs}");
        }
    }
}
