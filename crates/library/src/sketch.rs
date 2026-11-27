//! What a page draws while it reads: plates where its items will stand, so the answer takes their
//! places. A client lays the parts out as it lays out the real ones (how many fit is its own), shows them
//! once a read outlasts the core's `stage().quick_load_ms`, and fades the answer in over them.

/// A page that reads before it shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Reading {
    /// Home, a provider's shelves on a provider's profile.
    Home,
    Albums,
    Artists,
    Songs,
    Playlists,
    Favorites,
    Genres,
    Album,
    Artist,
    Playlist,
    /// A mix, a smart playlist, a genre's or a decade's songs: a list of songs under a head.
    Mix,
    /// A search's first answer.
    Search,
}

/// What stands for an item's picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Art {
    Square,
    /// An artist's.
    Round,
    /// A song's number on an album: no picture, a short bar.
    Number,
    /// A row of words alone (a genre).
    Bare,
}

/// One part of a loading page, top to bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum Sketch {
    /// A detail page's head: its picture, its name and its line.
    Head { art: Art },
    /// A shelf: a title bar over cards running past the page's edge.
    Shelf { art: Art },
    /// Cards across the page's width, row under row, to the page's end.
    Grid { art: Art },
    /// Rows of a list, each its picture beside two lines, to the page's end.
    Rows { art: Art },
}

/// The parts `page` shows while it reads. `head_known`: what opened the page already knew its head (a
/// card's name and cover), which then shows as itself.
#[cfg_attr(feature = "ffi", uniffi::export)]
pub fn sketch(page: Reading, head_known: bool) -> Vec<Sketch> {
    use Art::*;
    let head = |art| (!head_known).then_some(Sketch::Head { art });
    let body: &[Sketch] = match page {
        Reading::Home => &[Sketch::Shelf { art: Square }, Sketch::Shelf { art: Square }, Sketch::Shelf { art: Square }],
        Reading::Albums => &[Sketch::Grid { art: Square }],
        Reading::Artists => &[Sketch::Rows { art: Round }],
        Reading::Songs | Reading::Playlists | Reading::Search => &[Sketch::Rows { art: Square }],
        Reading::Favorites => &[Sketch::Shelf { art: Square }, Sketch::Rows { art: Square }],
        Reading::Genres => &[Sketch::Rows { art: Bare }],
        Reading::Album => &[Sketch::Rows { art: Number }],
        Reading::Artist => &[Sketch::Shelf { art: Square }, Sketch::Shelf { art: Square }],
        Reading::Playlist | Reading::Mix => &[Sketch::Rows { art: Square }],
    };
    let head = match page {
        Reading::Album | Reading::Playlist | Reading::Mix => head(Square),
        Reading::Artist => head(Round),
        _ => None,
    };
    head.into_iter().chain(body.iter().copied()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const EVERY: [Reading; 12] = [
        Reading::Home,
        Reading::Albums,
        Reading::Artists,
        Reading::Songs,
        Reading::Playlists,
        Reading::Favorites,
        Reading::Genres,
        Reading::Album,
        Reading::Artist,
        Reading::Playlist,
        Reading::Mix,
        Reading::Search,
    ];

    #[test]
    fn a_detail_page_sketches_its_head_only_when_nothing_knew_it() {
        for (page, art) in [(Reading::Album, Art::Square), (Reading::Artist, Art::Round), (Reading::Playlist, Art::Square), (Reading::Mix, Art::Square)] {
            assert_eq!(sketch(page, false)[0], Sketch::Head { art }, "{page:?}");
            assert_eq!(sketch(page, false)[1..], sketch(page, true)[..], "{page:?}");
            assert!(!sketch(page, true).iter().any(|s| matches!(s, Sketch::Head { .. })), "{page:?}");
        }
        assert_eq!(sketch(Reading::Album, true), [Sketch::Rows { art: Art::Number }], "an album's songs by number");
    }

    #[test]
    fn what_runs_to_the_page_end_comes_last_and_a_head_first() {
        for page in EVERY {
            for known in [false, true] {
                let parts = sketch(page, known);
                assert!(!parts.is_empty(), "{page:?}");
                let filling = parts.iter().position(|s| matches!(s, Sketch::Grid { .. } | Sketch::Rows { .. }));
                assert!(filling.is_none_or(|i| i == parts.len() - 1), "{page:?}: {parts:?}");
                assert!(parts.iter().skip(1).all(|s| !matches!(s, Sketch::Head { .. })), "{page:?}: {parts:?}");
            }
        }
    }
}
