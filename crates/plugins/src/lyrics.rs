//! Plugins as lyrics services: each enabled plugin with a `lyrics` export is asked, on its own thread,
//! in nori's race after nori's own services (nori_lyrics::race).

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use nori_lyrics::race::PluginLyrics;
use nori_lyrics::services::Lookup;
use nori_lyrics::trust::Named;
use nori_model::Song;

use crate::convert::song_out;
use crate::Plugins;

/// One plugin's lyrics service.
struct Service {
    plugins: Arc<Plugins>,
    id: String,
}

impl PluginLyrics for Service {
    fn plugin(&self) -> &str {
        &self.id
    }

    fn lookup<'a>(&'a self, song: &'a Song) -> Pin<Box<dyn Future<Output = Lookup> + Send + 'a>> {
        Box::pin(async move {
            let Some(w) = self.plugins.worker(&self.id) else { return Lookup::Unreachable };
            let s = song_out(song);
            match w.call(move |st, p| p.nori_plugin_lyrics().call_lookup(st, &s)).await {
                None => Lookup::Unreachable,
                Some(None) => Lookup::Missing,
                Some(Some(f)) => Lookup::Found(nori_lyrics::formats::from_lines(f.synced, f.lines.into_iter().map(|l| (l.start_ms as i64, l.words)).collect()), Named::default()),
            }
        })
    }
}

impl Plugins {
    /// The enabled plugins' lyrics services, in their order; empty, with nothing made, while none has one.
    pub fn lyrics_services(self: &Arc<Self>) -> Vec<Arc<dyn PluginLyrics>> {
        self.extending("lyrics").into_iter().map(|id| Arc::new(Service { plugins: self.clone(), id }) as Arc<dyn PluginLyrics>).collect()
    }
}
