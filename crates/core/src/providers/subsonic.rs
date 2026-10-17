//! A Subsonic server's profile (Navidrome, octo-fiesta): the [`Client`]'s requests, response cache and
//! offline write queue (client.rs, cache_policy.rs), its stream and cover URLs.

use std::sync::Arc;

use super::{FieldSpec, FormField, Provider, ProviderInfo};
use crate::browse::{profile_rules, ProfileRules};
use crate::cache_policy::{Page, Read, Stored};
use crate::client::{login_check, Client, NetResult, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::Transport;
use crate::Core;

pub(crate) struct Subsonic;

#[async_trait::async_trait]
impl Provider for Subsonic {
    fn info(&self) -> ProviderInfo {
        use FormField::*;
        let field = |what, advanced| FieldSpec { what, advanced };
        let fields = vec![
            field(Address, false),
            field(User, false),
            field(Password, false),
            field(Name, true),
            field(SecondAddress, true),
            field(ApiKey, true),
            field(Headers, true),
            field(LegacyAuth, true),
            field(SelfSigned, true),
            field(WifiOnly, true),
            field(ClientCert, true),
        ];
        ProviderInfo { kind: ServerKind::Subsonic, fields, sign_in: Vec::new() }
    }

    fn ready(&self, p: &SavedServer) -> bool {
        !p.url.trim().is_empty() && (!p.user.trim().is_empty() || !p.api_key.trim().is_empty())
    }

    /// Pings the address, else the second one; a server without token auth is signed in to with legacy
    /// auth, which the profile then keeps.
    async fn check(&self, transport: Arc<dyn Transport>, p: SavedServer) -> NetResult<SavedServer> {
        let legacy = login_check(transport, crate::server_config(&p), p.alt_url.clone()).await?;
        Ok(SavedServer { legacy_auth: legacy || p.legacy_auth, ..p })
    }

    fn rules(&self, _: &SavedServer, guest: bool) -> ProfileRules {
        profile_rules(guest)
    }

    fn configure(&self, core: &Core, profile: &SavedServer) -> nori_model::Result<()> {
        core.configure(crate::server_config(profile)).map(drop)
    }

    fn subsonic_api(&self) -> bool {
        true
    }

    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Page>> {
        client.subsonic_read(read).await.map(Some)
    }

    fn read_stored(&self, client: &Client, read: Read, within_ms: Option<i64>) -> NetResult<Stored> {
        client.subsonic_stored(read, within_ms)
    }

    async fn read_fetch(&self, client: &Client, read: Read, digest: Option<u64>) -> NetResult<Option<Page>> {
        client.subsonic_fetch(read, digest).await
    }

    async fn write(&self, client: &Client, w: Write) -> NetResult<()> {
        client.subsonic_write(w).await
    }

    fn stream(&self, client: &Client, id: String, q: StreamQuality) -> StreamTarget {
        let key = crate::stream::stream_cache_key(&id, &q);
        StreamTarget { url: client.core.stream_url(id, q.bit_rate, q.format), key }
    }

    fn cover_address(&self, core: &Core, cover: &str, size: u32) -> String {
        let mut out = String::new();
        crate::covers::cover_url_into(&mut out, &core.url_prefix("getCoverArt".into()), cover, size as i32);
        out
    }

    fn cover_prefix(&self, core: &Core) -> Option<String> {
        Some(core.url_prefix("getCoverArt".into()))
    }
}
