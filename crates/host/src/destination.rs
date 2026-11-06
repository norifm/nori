//! Where the music plays (`nori_core::destination`), from the session's remote control and plugin cast.

use nori_core::destination::{destination, Destination};

use crate::session::Handle;

impl Handle {
    /// Where the music plays now.
    pub fn destination(&self) -> Destination {
        #[cfg(feature = "desktop")]
        let cast = self.plugins.as_ref().and_then(|p| p.cast()).as_ref().map(Into::into);
        #[cfg(not(feature = "desktop"))]
        let cast = None;
        destination(self.remotes.get(), cast)
    }
}
