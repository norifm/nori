//! What installing a plugin keeps of the one it replaces, for both plugin hosts (crates/plugins and
//! the web's).

use serde::{Deserialize, Serialize};

/// A plugin's place among the installed and the user's choices about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Standing {
    /// Turned on.
    pub on: bool,
    /// Kept at its version: no update is offered.
    pub held: bool,
    /// Its place in the order plugins apply in.
    pub place: u32,
}

/// The standing of a plugin just installed over `had` (None for a new plugin): an update keeps the one
/// it had, and its stored values; a new plugin goes after the others (`next_place`), held by nothing, on
/// when `on_when_new`.
pub fn installed_over(had: Option<Standing>, next_place: u32, on_when_new: bool) -> Standing {
    had.unwrap_or(Standing { on: on_when_new, held: false, place: next_place })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_update_keeps_its_standing_and_a_new_plugin_goes_last() {
        let had = Standing { on: false, held: true, place: 1 };
        assert_eq!(installed_over(Some(had), 4, true), had);
        assert_eq!(installed_over(None, 4, true), Standing { on: true, held: false, place: 4 });
        assert_eq!(installed_over(None, 0, false), Standing { on: false, held: false, place: 0 }, "restored off");
    }
}
