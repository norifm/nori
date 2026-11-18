//! Display facts derived from settings and the queue; clients word them.

use nori_model::deciders::{NoriButton, SessionButtons};

use crate::Core;

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// [`session_buttons`] for the queue's current song.
    pub fn session_buttons_now(&self, starred: bool, shuffle: bool) -> SessionButtons {
        let song = self.session.playlist(|p| p.current_id().is_some_and(|id| !crate::queue::is_radio(id)));
        session_buttons(song, starred, shuffle)
    }
}

/// The media session's own buttons: the seal and a radio from the song for a library song, shuffle and
/// repeat always; the seal and shuffle first, in the places other players put them.
pub fn session_buttons(song: bool, starred: bool, shuffle: bool) -> SessionButtons {
    let buttons = if song { vec![NoriButton::Favorite, NoriButton::Shuffle, NoriButton::Repeat, NoriButton::Radio] } else { vec![NoriButton::Shuffle, NoriButton::Repeat] };
    SessionButtons { buttons, starred: song && starred, shuffling: shuffle }
}

/// A radio stream's title: the announced ICY title, else the station name.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn radio_title(announced: Option<String>, station: Option<String>) -> Option<String> {
    announced.filter(|a| !a.trim().is_empty()).or(station)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown() {
        let b = session_buttons(false, true, false);
        assert_eq!((b.buttons, b.starred), (vec![NoriButton::Shuffle, NoriButton::Repeat], false), "no seal or radio without a library song");
        assert_eq!(session_buttons(true, true, false).buttons, [NoriButton::Favorite, NoriButton::Shuffle, NoriButton::Repeat, NoriButton::Radio]);

        // Radio title prefers announcement.
        let s = |v: &str| Some(v.to_string());
        assert_eq!(radio_title(s("Artist - Song"), s("FM 4")), s("Artist - Song"));
        assert_eq!(radio_title(s("  "), s("FM 4")), s("FM 4"));
        assert_eq!(radio_title(None, s("FM 4")), s("FM 4"));
        assert_eq!(radio_title(None, None), None);
    }

}
