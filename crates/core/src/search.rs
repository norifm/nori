//! Search from the index and the server. Result splitting is nori-library's.

use crate::{Core, Result, SearchResult};

pub use nori_library::search::*;

/// Live search state: each keystroke is answered from the index, and the server (which also knows
/// provider items) once typing pauses. Answers for a stale query are dropped, and an index answer never
/// replaces the server's for the same query.
#[derive(Default)]
#[cfg_attr(feature = "ffi", derive(uniffi::Object))]
pub struct SearchSession(parking_lot::Mutex<Session>);

#[cfg_attr(feature = "ffi", uniffi::export)]
impl SearchSession {
    #[cfg_attr(feature = "ffi", uniffi::constructor)]
    pub fn new() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::default())
    }

    /// The field now holds `text`; blank shows the recent searches. The same query keeps its answer.
    pub fn typed(&self, text: String) -> SearchView {
        let mut s = self.0.lock();
        let same = s.query() == text.trim();
        s.text = text;
        if s.query().is_empty() {
            s.answer = Answer::Blank;
        } else if !same {
            s.answer = Answer::Asking(s.answer.take_split());
        }
        s.view()
    }

    /// Applies the index's answer to `query` unless stale or the server already answered.
    pub fn local(&self, core: std::sync::Arc<Core>, query: String, limit: u32) -> Result<Option<SearchView>> {
        let split = core.local_search_split(query.clone(), limit)?;
        let mut s = self.0.lock();
        if s.query() != query {
            return Ok(None);
        }
        match &mut s.answer {
            Answer::Asking(index) | Answer::Failed { index, .. } => *index = Some(split),
            Answer::Blank | Answer::Server(_) => return Ok(None),
        }
        Ok(Some(s.view()))
    }

    /// Applies the server's answer to `query`; None if stale.
    pub fn server(&self, query: String, result: SearchResult) -> Option<SearchView> {
        let split = search_split(result, &query);
        let mut s = self.0.lock();
        if s.query() != query {
            return None;
        }
        s.answer = Answer::Server(split);
        Some(s.view())
    }

    /// Asks the server and applies the answer ([`SearchSession::server`]); an error when the request fails.
    pub async fn ask(&self, client: std::sync::Arc<crate::client::Client>, query: String) -> crate::client::NetResult<Option<SearchView>> {
        let sizes = crate::browse::library_sizes();
        let read = crate::cache_policy::Read::Search { query: query.clone(), songs: sizes.search_songs, albums: sizes.search_albums, artists: sizes.search_artists };
        let crate::cache_policy::Page::Found { v } = client.read_now(read).await? else { return Ok(None) };
        Ok(self.server(query, v))
    }

    /// The server failed for `query`: keeps the index answer and reports the fallback. None if stale, or
    /// another ask for it was answered.
    pub fn failed(&self, query: String, reason: Option<String>) -> Option<SearchView> {
        let mut s = self.0.lock();
        if s.query() != query || matches!(s.answer, Answer::Blank | Answer::Server(_)) {
            return None;
        }
        let index = s.answer.take_split();
        s.answer = Answer::Failed { index, fallback: SearchFallback { reason } };
        Some(s.view())
    }

    pub fn scope(&self, scope: SearchScope) -> SearchView {
        let mut s = self.0.lock();
        s.scope = scope;
        s.view()
    }
}

impl SearchSession {
    /// [`SearchSession::typed`], then the index's answer to the query at once ([`SearchSession::local`]).
    pub fn typed_with_index(&self, core: std::sync::Arc<Core>, text: String) -> SearchView {
        let view = self.typed(text);
        if view.query.is_empty() {
            return view;
        }
        let limit = crate::browse::library_sizes().local_search;
        self.local(core, view.query.clone(), limit).ok().flatten().unwrap_or(view)
    }

    /// [`SearchSession::ask`], a failure kept as the fallback `reason` words: the view to show, None when
    /// it is stale.
    pub async fn answer(&self, client: std::sync::Arc<crate::client::Client>, query: String, reason: fn(&crate::transport::NetError) -> String) -> Option<SearchView> {
        match self.ask(client, query.clone()).await {
            Ok(v) => v,
            Err(e) => self.failed(query, Some(reason(&e))),
        }
    }
}

#[cfg_attr(feature = "ffi", uniffi::export)]
impl Core {
    /// Remembers `query` and returns the history; None (not remembered) when too short.
    pub fn search_remember_recent(&self, query: String) -> Result<Option<Vec<String>>> {
        if query.encode_utf16().count() < REMEMBER_MIN_UTF16 {
            return Ok(None);
        }
        self.search_remember(query)?;
        Ok(Some(self.search_history()?))
    }
}

impl Core {
    /// Index search, ranked and split like the server's.
    pub(crate) fn local_search_split(&self, query: String, limit: u32) -> Result<SearchSplit> {
        let found = self.local_search(query.clone(), limit)?;
        Ok(split(ranked(found, &query)))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{Album, Artist, Song};

    fn result() -> SearchResult {
        let s = |id: &str, ext: bool| Song { id: id.into(), is_external: ext, ..Default::default() };
        let a = |id: &str, ext: bool| Album { id: id.into(), is_external: ext, ..Default::default() };
        SearchResult {
            artists: vec![Artist { id: "ar".into(), ..Default::default() }],
            albums: vec![a("al", false), a("ext-al", true), a("ext-al", true)],
            songs: vec![s("1", false), s("ext-2", true), s("1", false), s("3", false)],
            playlists: Vec::new(),
        }
    }

    #[test]
    fn search_session() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        let s = SearchSession::new();
        let v = s.typed(" dogs ".into());
        assert_eq!(v.query, "dogs");
        assert!(matches!(v.answer, SearchShown::Asking { index: None }));
        assert!(s.local(core.clone(), "dog".into(), 30).unwrap().is_none(), "stale");
        assert!(s.local(core.clone(), "dogs".into(), 30).unwrap().is_some());
        let v = s.server("dogs".into(), result()).unwrap();
        assert!(v.scopes_offered);
        let SearchShown::Found { result: found } = &v.answer else { panic!("{:?}", v.answer) };
        assert_eq!(found.songs.len(), 3);
        assert!(s.local(core, "dogs".into(), 30).unwrap().is_none(), "server answer stands");
        assert_eq!(s.scope(SearchScope::Providers).answer.result().unwrap().songs.len(), 1);
        assert_eq!(s.scope(SearchScope::Library).answer.result().unwrap().songs.len(), 2);
        assert!(s.server("cats".into(), result()).is_none());
        s.typed("cats".into());
        let failed = s.failed("cats".into(), Some("timeout".into())).unwrap();
        let SearchShown::Failed { index, fallback } = &failed.answer else { panic!("{:?}", failed.answer) };
        assert_eq!(*fallback, SearchFallback { reason: Some("timeout".into()) });
        assert!(index.is_some(), "the last answer stays");
        let v = s.typed("  ".into());
        assert!(matches!(v.answer, SearchShown::Blank));
        assert!(!s.scope(SearchScope::Everything).scopes_offered);
        let none = s.typed("x".into());
        assert!(matches!(none.answer, SearchShown::Asking { .. }));
        let v = s.server("x".into(), SearchResult::default()).unwrap();
        assert!(matches!(v.answer, SearchShown::NothingFound));
        let only_playlists = SearchResult { playlists: vec![crate::Playlist { id: "yt-VLa".into(), ..Default::default() }], ..Default::default() };
        assert!(matches!(s.server("x".into(), only_playlists).unwrap().answer, SearchShown::Found { .. }));
        assert_eq!(s.scope(SearchScope::Library).answer.result().map_or(0, |r| r.songs.len()), 0);

        // Short queries are not remembered.
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        assert_eq!(core.search_remember_recent("a".into()).unwrap(), None);
        assert!(core.search_history().unwrap().is_empty());
        // One emoji is two UTF-16 units.
        assert_eq!(core.search_remember_recent("🎵".into()).unwrap(), Some(vec!["🎵".to_string()]));
        let history = core.search_remember_recent("dogs".into()).unwrap().unwrap();
        assert_eq!(history.len(), 2);
        assert!(history.contains(&"dogs".to_string()));
    }

    #[test]
    fn a_failed_ask_leaves_the_server_answer_standing() {
        // Two asks for one query overlap: the first answers, the second fails.
        let s = SearchSession::new();
        s.typed("dogs".into());
        s.server("dogs".into(), result()).unwrap();
        assert!(s.failed("dogs".into(), Some("timeout".into())).is_none());
        let v = s.scope(SearchScope::Everything);
        assert!(matches!(&v.answer, SearchShown::Found { result } if result.songs.len() == 3));
    }

    #[test]
    fn spaces_around_the_query_keep_its_answer() {
        let s = SearchSession::new();
        s.typed("dogs".into());
        s.server("dogs".into(), result()).unwrap();
        let v = s.typed("dogs ".into());
        assert_eq!(v.text, "dogs ");
        assert!(matches!(v.answer, SearchShown::Found { .. }), "the query is the same, so nothing asks it again");

        s.typed("cats".into());
        s.failed("cats".into(), None).unwrap();
        let v = s.typed(" cats".into());
        assert!(matches!(v.answer, SearchShown::Failed { .. }));
    }

    #[test]
    fn recent_searches_keep_the_newest_and_forget_one() {
        let core = Core::new(String::new(), "t".into(), Default::default()).unwrap();
        // Older searches, newest last.
        let older: Vec<String> = (0..SEARCHES_KEPT + 2).map(|i| format!("query {i}")).collect();
        for (ts, q) in older.iter().enumerate() {
            core.db.lock().execute("INSERT INTO searches(server, query, ts) VALUES(sid(), ?1, ?2)", rusqlite::params![q, ts as i64]).unwrap();
        }
        // A profile that kept more before the cap still reads only the newest.
        assert_eq!(core.search_history().unwrap().len(), SEARCHES_KEPT as usize);
        core.search_remember("newest".into()).unwrap();
        let newest: Vec<String> = std::iter::once("newest".to_string()).chain(older.iter().rev().take(SEARCHES_KEPT as usize - 1).cloned()).collect();
        assert_eq!(core.search_history().unwrap(), newest);
        let stored: i64 = core.db.lock().query_row("SELECT COUNT(*) FROM searches", [], |r| r.get(0)).unwrap();
        assert_eq!(stored, SEARCHES_KEPT as i64);

        core.search_forget_one(newest[1].clone()).unwrap();
        let after = core.search_history().unwrap();
        assert_eq!(after.len(), newest.len() - 1);
        assert!(!after.contains(&newest[1]) && after.contains(&newest[0]));
    }

}
