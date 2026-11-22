//! Adding a plugin repo from what the user typed, and the names repos go by.

use std::sync::{Arc, Mutex};

use nori_net::transport::{Exchange, FailureKind, Network, Transport, TransportError, TransportResponse};
use nori_plugins::{Plugins, Repo, RepoAdd, RepoProblem, Side, Value, OFFICIAL};

/// Answers the URLs it serves with their status and body, any other with no connection, noting each URL.
#[derive(Default)]
struct Web {
    serves: Mutex<Vec<(String, u16, Vec<u8>)>>,
    asked: Mutex<Vec<String>>,
}

impl Web {
    fn serve(&self, url: &str, status: u16, body: &str) {
        self.serves.lock().unwrap().push((url.into(), status, body.as_bytes().to_vec()));
    }
}

#[async_trait::async_trait]
impl Transport for Web {
    async fn get(&self, url: String, _: u32) -> Result<TransportResponse, TransportError> {
        self.asked.lock().unwrap().push(url.clone());
        match self.serves.lock().unwrap().iter().find(|(u, ..)| *u == url) {
            Some((_, status, body)) => Ok(TransportResponse { status: *status, body: body.clone(), headers: Vec::new() }),
            None => Err(TransportError::Failed { kind: FailureKind::UnknownHost, detail: None }),
        }
    }

    async fn send(&self, e: Exchange) -> Result<TransportResponse, TransportError> {
        self.get(e.url, e.timeout_ms).await
    }

    fn address_changed(&self) {}

    fn network(&self) -> Network {
        Network::Unmetered
    }
}

struct Quiet;

impl Side for Quiet {
    fn stored(&self, _: &str) -> Option<Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn laid(&self, _: &str) {}
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, _: &str, _: &[String]) {}
}

const EMPTY: &str = r#"{"plugins": []}"#;
const MAIN: &str = "https://raw.githubusercontent.com/someone/plugins/main/index.json";
const MASTER: &str = "https://raw.githubusercontent.com/someone/plugins/master/index.json";
const PAGES: &str = "https://someone.github.io/plugins/index.json";

fn open(dir: &nori_testdir::TempDir, web: &Arc<Web>) -> Arc<Plugins> {
    let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(Quiet));
    plugins.set_transport(Some(web.clone()));
    plugins
}

fn add(plugins: &Plugins, typed: &str) -> RepoAdd {
    nori_tasks::block_on(plugins.add_repo(typed.into()))
}

fn named(plugins: &Plugins) -> Vec<(String, Option<String>)> {
    plugins.repos().into_iter().map(|r| (r.url, r.name)).collect()
}

#[test]
fn a_user_repo_is_found_where_nori_plugins_keeps_its_index() {
    let dir = nori_testdir::TempDir::new("repos-shorthand");
    let web = Arc::new(Web::default());
    web.serve(MAIN, 404, "Not Found");
    web.serve(MASTER, 200, r#"{"name": "Someone's plugins", "plugins": []}"#);
    let plugins = open(&dir, &web);
    let want = Repo { url: MASTER.into(), official: false, name: Some("Someone's plugins".into()) };
    assert_eq!(add(&plugins, "someone/plugins"), RepoAdd::Added { repo: want.clone() }, "called as its index names it");
    assert_eq!(*web.asked.lock().unwrap(), [MAIN, MASTER], "main first, and no further than the first that reads");
    assert_eq!(add(&plugins, "https://github.com/someone/plugins"), RepoAdd::Added { repo: want }, "found again, not added twice");
    assert_eq!(plugins.repos().len(), 2);
    assert_eq!(add(&plugins, "norifm/nori-plugins"), RepoAdd::Added { repo: Repo { url: OFFICIAL.into(), official: true, name: None } }, "nori's own is there already");
    assert_eq!(web.asked.lock().unwrap().len(), 3, "and isn't asked");
}

#[test]
fn adding_says_why_no_repo_is_there() {
    let web = Arc::new(Web::default());
    web.serve(MAIN, 404, "");
    web.serve(MASTER, 404, "");
    web.serve(PAGES, 200, "<html>");
    web.serve("https://down.example/index.json", 503, "");
    web.serve("https://gone.example/index.json", 404, "");
    for (typed, problem) in [
        ("someone/plugins", RepoProblem::NotARepo),
        ("https://down.example/index.json", RepoProblem::Unreachable),
        ("https://gone.example/index.json", RepoProblem::NotFound),
        ("https://nowhere.example/index.json", RepoProblem::Unreachable),
        ("ftp://friend.example/index.json", RepoProblem::NotFound),
    ] {
        let dir = nori_testdir::TempDir::new("repos-problem");
        let plugins = open(&dir, &web);
        assert_eq!(add(&plugins, typed), RepoAdd::Failed { problem }, "{typed}");
        assert_eq!(plugins.repos().len(), 1, "{typed}: nothing added");
    }
    let dir = nori_testdir::TempDir::new("repos-no-transport");
    let plugins = Plugins::open(dir.path().to_path_buf(), Arc::new(Quiet));
    assert_eq!(add(&plugins, "someone/plugins"), RepoAdd::Failed { problem: RepoProblem::Unreachable }, "no connection yet");
}

#[test]
fn a_repos_name_is_kept_and_an_empty_one_gives_back_its_own() {
    let dir = nori_testdir::TempDir::new("repos-named");
    let web = Arc::new(Web::default());
    let friend = "https://friend.example/nori/index.json";
    web.serve(friend, 200, EMPTY);
    let plugins = open(&dir, &web);
    add(&plugins, "friend.example/nori/index.json");
    assert_eq!(named(&plugins)[1], (friend.to_string(), Some("friend.example".to_string())), "with no name, its host");
    plugins.rename_repo(friend.into(), "  Friend  ".into());
    plugins.rename_repo(OFFICIAL.into(), "Mine".into());
    assert_eq!(named(&open(&dir, &web)), [(OFFICIAL.to_string(), None), (friend.to_string(), Some("Friend".to_string()))], "kept, and nori's own keeps its name");

    let stored: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("index.json")).unwrap()).unwrap();
    assert_eq!((&stored["repos"], &stored["repo_names"]), (&serde_json::json!([friend]), &serde_json::json!({ friend: "Friend" })), "the addresses as an older nori reads them, the names beside");

    plugins.rename_repo(friend.into(), " ".into());
    assert_eq!(named(&plugins)[1].1.as_deref(), Some("friend.example"));
    web.serves.lock().unwrap()[0].2 = br#"{"name": "Friend's plugins", "plugins": []}"#.to_vec();
    let offers = nori_tasks::block_on(plugins.offers());
    assert_eq!(offers.failed.iter().map(|f| (f.repo.url.as_str(), f.problem)).collect::<Vec<_>>(), [(OFFICIAL, RepoProblem::Unreachable)]);
    assert_eq!(named(&plugins)[1].1.as_deref(), Some("Friend's plugins"), "unnamed, it takes its index's name when read");
    plugins.rename_repo(friend.into(), "Friend".into());
    nori_tasks::block_on(plugins.offers());
    assert_eq!(named(&plugins)[1].1.as_deref(), Some("Friend"), "a name given stays over its index's");

    plugins.remove_repo(friend.into());
    add(&plugins, friend);
    assert_eq!(named(&plugins)[1].1.as_deref(), Some("Friend's plugins"), "removed, its name went with it");
}

#[test]
fn repos_an_older_nori_kept_read_with_their_short_names() {
    let dir = nori_testdir::TempDir::new("repos-older");
    std::fs::write(dir.path().join("index.json"), r#"{"plugin": [], "repos": ["https://raw.githubusercontent.com/a/b/main/index.json"], "checked_ms": 0}"#).unwrap();
    let plugins = open(&dir, &Arc::new(Web::default()));
    assert_eq!(named(&plugins)[1].1.as_deref(), Some("b"));
    assert_eq!(nori_plugins::repo_called("https://raw.githubusercontent.com/a/b/main/index.json".into(), plugins.repos()).as_deref(), Some("b"));
    assert_eq!(nori_plugins::repo_called("https://gone.example/index.json".into(), plugins.repos()).as_deref(), Some("gone.example"), "a repo no longer listed, by its address");
    assert_eq!(nori_plugins::repo_called(OFFICIAL.into(), plugins.repos()), None);
}
