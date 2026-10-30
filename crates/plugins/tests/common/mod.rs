//! What the extension points' tests share: the test plugin installed and on, over a side that notes
//! what it shows.

use std::sync::{Arc, Mutex};

use nori_plugins::testing::{fixture, FIXTURE};
use nori_plugins::{Plugins, Side, Value};

/// Notes the notices the plugin shows: words and args.
#[derive(Default)]
pub struct Noted {
    pub notices: Mutex<Vec<(String, Vec<String>)>>,
}

impl Side for Noted {
    fn stored(&self, _: &str) -> Option<Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<Value> {
        None
    }
    fn laid(&self, _: &str) {}
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, words: &str, args: &[String]) {
        self.notices.lock().unwrap().push((words.to_string(), args.to_vec()));
    }
}

/// The test plugin built with `features`, installed under `dir` and on.
#[allow(dead_code)]
pub fn plugged(dir: &nori_testdir::TempDir, features: &str) -> (Arc<Plugins>, Arc<Noted>) {
    let side = Arc::new(Noted::default());
    let plugins = Plugins::open(dir.path().to_path_buf(), side.clone());
    plugins.install(&fixture(features)).unwrap();
    plugins.turn(FIXTURE.into(), true);
    (plugins, side)
}
