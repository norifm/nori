//! A Subsonic server's profile (Navidrome, octo-fiesta): each read as an endpoint and its answer as a
//! page (reads.rs, wire.rs), kept raw and read again; the server's addresses, signing, writes and their
//! offline queue, and the library walk (server.rs); its stream and cover URLs.

mod reads;
mod server;
pub(crate) mod wire;

use std::sync::Arc;

use super::{Answer, FieldSpec, FormField, Provider, ProviderInfo};
use crate::browse::{profile_rules, ProfileRules};
use crate::cache_policy::{Page, Read};
use crate::client::{Client, NetResult, Write};
use crate::settings::{SavedServer, ServerKind};
use crate::stream::{StreamQuality, StreamTarget};
use crate::transport::{NetError, Transport};
use crate::Core;

pub use server::login_check;

/// The Subsonic error for an item the server does not have.
const NOT_FOUND: i32 = 70;

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

    /// The server's answer, kept raw; a server has no home feed.
    async fn read(&self, client: &Client, read: Read) -> NetResult<Option<Answer>> {
        let Some((endpoint, params, _)) = reads::request(&read) else { return Ok(None) };
        Ok(Some(Answer::Raw(client.fetch(endpoint, params).await?)))
    }

    fn parse(&self, client: &Client, read: &Read, raw: &[u8]) -> NetResult<Page> {
        reads::page(client, read, raw)
    }

    fn scope(&self, client: &Client, read: &Read) -> String {
        reads::scope(client, read)
    }

    fn gone(&self, e: &NetError) -> bool {
        matches!(e, NetError::Api { code: NOT_FOUND, .. })
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
        crate::covers::cover_url_into(&mut out, &core.url_prefix("getCoverArt"), cover, size as i32);
        out
    }

    fn cover_prefix(&self, core: &Core) -> Option<String> {
        Some(core.url_prefix("getCoverArt"))
    }
}
