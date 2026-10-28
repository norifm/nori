// Host perf of a plugin's sound stage, included into tests/engine.rs beside perf_bench.rs: the test
// plugin's stage (crates/plugins/fixture) through wasmtime, against the same music with none and with
// nori's equalizer. This checkout only (tools/perf-host.sh compares perf_bench.rs's cases):
//
//   cargo test --release -p nori-engine --test engine perf_stage -- --ignored --nocapture --test-threads=1

/// The plugin host's session, which hears nothing.
struct Unheard;

impl nori_plugins::Side for Unheard {
    fn stored(&self, _: &str) -> Option<nori_plugins::Value> {
        None
    }
    fn effective(&self, _: &str) -> Option<nori_plugins::Value> {
        None
    }
    fn laid(&self, _: &str) {}
    fn status(&self, _: &str, _: &str, _: &str, _: &[String]) {}
    fn shown(&self, _: &str, _: &str, _: bool, _: bool) {}
    fn notice(&self, _: &str, _: bool, _: &str, _: &[String]) {}
}

#[test]
#[ignore]
fn perf_stage() {
    crate::perf_alloc::exclude_this_thread();
    let dir = nori_testdir::TempDir::new("perf-stage");
    let plugins = nori_plugins::Plugins::open(dir.path().to_path_buf(), std::sync::Arc::new(Unheard));
    plugins.install(&nori_plugins::testing::fixture("")).unwrap();
    plugins.turn(nori_plugins::testing::FIXTURE.into(), true);
    // Its gain grows with each block; held at a quiet one, the music stays in range.
    plugins.set(nori_plugins::testing::FIXTURE.into(), "gain".into(), nori_plugins::Value::Level(0.0));
    let stages = plugins.stages();
    let long = music(300.0, 91);
    let one = [("a", &long[..])];
    perf_case("plain", &one, perf_app(prefs_off()), Settings::default(), 4);
    perf_case("equalizer", &one, perf_app(prefs_off()), loud_eq(), 4);
    perf_case("stage", &one, perf_app(prefs_off()), Settings { stages, ..Settings::default() }, 4);
}
