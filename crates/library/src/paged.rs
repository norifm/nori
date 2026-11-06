//! A list read a page at a time, for every client: one page on its way at a time, none asked for after
//! the last, a page that could not be read asked for again, and a list started over (another order or
//! filter) dropping the page on its way.
//!
//! [`Paged`] is the state over any cursor (an offset, or the history's [`crate::browse::HistoryAfter`]);
//! [`Pages`] is the same over page numbers, for a client that keeps its own cursor (Android).

/// Where a paged list stands, for the page to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "ffi", derive(uniffi::Enum))]
pub enum PageState {
    /// More may come; none is on its way.
    More,
    /// A page is on its way.
    Reading,
    /// The last page asked could not be read; the next ask asks it again.
    Failed,
    /// Every page is in.
    Read,
}

#[derive(Debug, Clone, PartialEq)]
enum Paging<C> {
    More(C),
    Reading(C),
    Failed(C),
    Read,
}

/// A list read a page at a time from cursor `C`, starting at its first page.
#[derive(Debug, Clone, PartialEq)]
pub struct Paged<C> {
    paging: Paging<C>,
    first: C,
    /// The page answered last, which may answer again (the stored page, then the server's).
    answered: Option<C>,
}

impl<C: Clone + PartialEq> Paged<C> {
    pub fn new(first: C) -> Self {
        Paged { paging: Paging::More(first.clone()), first, answered: None }
    }

    /// The page to read next, now on its way: none while one is, or after the last.
    pub fn ask(&mut self) -> Option<C> {
        match &self.paging {
            Paging::More(at) | Paging::Failed(at) => {
                let at = at.clone();
                self.paging = Paging::Reading(at.clone());
                Some(at)
            }
            Paging::Reading(_) | Paging::Read => None,
        }
    }

    /// Page `from` came, with `next` after it (none: it was the last). True when it belongs to the list
    /// as it stands: the page on its way, or the one answered last answering again, which then says
    /// again what follows unless a later page is on its way.
    pub fn answered(&mut self, from: &C, next: Option<C>) -> bool {
        let settles = match &self.paging {
            Paging::Reading(at) => at == from,
            Paging::More(_) | Paging::Read | Paging::Failed(_) => self.answered.as_ref() == Some(from),
        };
        if settles {
            self.paging = next.map_or(Paging::Read, Paging::More);
            self.answered = Some(from.clone());
            return true;
        }
        self.answered.as_ref() == Some(from)
    }

    /// Page `from` could not be read: it is asked again with the next [`Paged::ask`]. A page answered
    /// already (a stored one, whose server copy failed) stays read.
    pub fn failed(&mut self, from: &C) {
        if matches!(&self.paging, Paging::Reading(at) if at == from) {
            self.paging = Paging::Failed(from.clone());
        }
    }

    /// From the first page again; the page on its way is dropped.
    pub fn reset(&mut self) {
        self.paging = Paging::More(self.first.clone());
        self.answered = None;
    }

    /// Nothing more is read (the list was emptied); the page on its way is dropped.
    pub fn end(&mut self) {
        self.paging = Paging::Read;
        self.answered = None;
    }

    pub fn state(&self) -> PageState {
        match self.paging {
            Paging::More(_) => PageState::More,
            Paging::Reading(_) => PageState::Reading,
            Paging::Failed(_) => PageState::Failed,
            Paging::Read => PageState::Read,
        }
    }

    /// Whether more may come after what is listed: false once the last page is in.
    pub fn more(&self) -> bool {
        self.paging != Paging::Read
    }
}

/// [`Paged`] over page numbers, for a client that keeps its own cursor: [`Pages::ask`] says which page
/// to read (0 the first), the answer comes back with that number.
#[derive(Debug)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct Pages {
    paged: std::sync::Mutex<Paged<u32>>,
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Pages {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Pages { paged: std::sync::Mutex::new(Paged::new(0)) })
    }

    /// The page to read now, or none while one is on its way or after the last.
    pub fn ask(&self) -> Option<u32> {
        self.paged().ask()
    }

    /// Page `page` came; `more` when pages follow it.
    pub fn answered(&self, page: u32, more: bool) {
        self.paged().answered(&page, more.then_some(page + 1));
    }

    pub fn failed(&self, page: u32) {
        self.paged().failed(&page);
    }

    pub fn reset(&self) {
        self.paged().reset();
    }

    pub fn end(&self) {
        self.paged().end();
    }

    pub fn state(&self) -> PageState {
        self.paged().state()
    }
}

impl Pages {
    fn paged(&self) -> std::sync::MutexGuard<'_, Paged<u32>> {
        self.paged.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_page_at_a_time_until_the_last() {
        let mut p = Paged::new(0u32);
        assert_eq!(p.ask(), Some(0));
        assert_eq!(p.ask(), None, "one on its way");
        assert_eq!(p.state(), PageState::Reading);
        assert!(p.answered(&0, Some(50)));
        assert_eq!(p.ask(), Some(50));
        assert!(p.answered(&50, None));
        assert_eq!(p.ask(), None, "after the last");
        assert!(!p.more());
    }

    #[test]
    fn a_failed_page_is_asked_again() {
        let mut p = Paged::new(0u32);
        p.ask();
        p.answered(&0, Some(50));
        p.ask();
        p.failed(&50);
        assert_eq!(p.state(), PageState::Failed);
        assert!(p.more(), "a failure is not the end");
        assert_eq!(p.ask(), Some(50));
    }

    #[test]
    fn a_reset_drops_the_page_on_its_way() {
        let mut p = Paged::new(0u32);
        p.ask();
        p.answered(&0, Some(50));
        p.ask();
        p.reset();
        assert!(!p.answered(&50, None), "the old list's page");
        assert_eq!(p.ask(), Some(0));
        p.failed(&50);
        assert_eq!(p.state(), PageState::Reading, "an old failure changes nothing");
    }

    #[test]
    fn a_page_answered_twice_says_again_what_follows() {
        let mut p = Paged::new(0u32);
        p.ask();
        // The stored page, full; then the server's, shorter.
        assert!(p.answered(&0, Some(50)));
        assert!(p.answered(&0, None));
        assert_eq!(p.ask(), None);
        // Read again from the top: the stored page short, the server's full.
        p.reset();
        p.ask();
        p.answered(&0, None);
        assert!(p.answered(&0, Some(50)));
        assert_eq!(p.ask(), Some(50));
        // With the next page on its way, the first answering again is still the list's, and asks nothing.
        assert!(p.answered(&0, None));
        assert_eq!(p.state(), PageState::Reading);
        p.failed(&0);
        assert_eq!(p.state(), PageState::Reading, "the stored copy stands");
    }

    #[test]
    fn ended_lists_ask_nothing() {
        let mut p = Paged::new(None::<i64>);
        p.ask();
        p.end();
        assert!(!p.answered(&None, Some(Some(3))));
        assert_eq!(p.ask(), None);
        p.reset();
        assert_eq!(p.ask(), Some(None));
    }

    #[test]
    fn pages_count_from_the_first() {
        let p = Pages::new();
        assert_eq!(p.ask(), Some(0));
        p.answered(0, true);
        assert_eq!(p.ask(), Some(1));
        p.failed(1);
        assert_eq!(p.ask(), Some(1));
        p.answered(1, false);
        assert_eq!(p.ask(), None);
        assert_eq!(p.state(), PageState::Read);
    }
}
