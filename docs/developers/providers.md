# Providers

Applies to nori 0.7, every client. The web client and the iPod build no plugin provider.

A profile plays from a **provider**: a Subsonic server (Navidrome, octo-fiesta), or a plugin's (YouTube
Music is one: [Plugins](plugins.md)). Each provider is one implementation of the `Provider` trait in
`crates/core/src/providers/`. Nothing else in the core, the engine or a client names one: they ask the
profile's provider and draw what it describes.

This is not octo-fiesta's provider items (the `ext-` songs a Subsonic server fetches from elsewhere).

| provider | module | built by |
|---|---|---|
| Subsonic | `providers/subsonic/` | every client |
| a plugin's | `providers/plugin.rs`: the trait answered by the plugin's `provider` export (`wit/plugin.wit`) | nori-core's `plugins` feature: Android, desktop, terminal |
| a LAN jam host's | `providers/hosted.rs`: a guest profile's reads and stream lookups asked of the host's door, which answers them with its own provider | every native client |

The kinds a client offers are a `Kinds`' (`Kinds::builtin`, or `Kinds::with_plugins` over the plugin
host): Subsonic first, then each enabled plugin with a provider, named by the plugin's own `name`
string (`Kinds::plugin_name`). A profile of a plugin kind whose plugin is missing doesn't open, except a
LAN jam's guest profile: it keeps the host's kind (its songs are that provider's) and plays through the
host (`HostedProvider`), so a guest needs neither the plugin nor an account.

## What a provider answers

`nori_core::providers::Provider`. One instance is made per profile opened (`Core::configure_profile`),
so it can hold that profile's state (a session, resolved streams).

| method | what |
|---|---|
| `info` | the add form's fields (`FieldSpec`: a `FormField` such as name, address, user; plain or advanced) and how the account signs in (`SignIn::WebPage { start_url }`, `SignIn::Paste`) |
| `ready`, `check` | whether the form can be sent; checking a profile before it is kept (a server is pinged) |
| `rules` | the profile's `ProfileRules`: library sections, favorites, downloads, radio, remote control and jams, scrobbles, its own Home feed, and the same for a jam guest |
| `configure` | points the `Core` at the profile |
| `read`, `parse`, `scope`, `gone` | the core's `Read`s (search, album, artist, playlist, Home feed, library sections) as a `Page` (`Answer::Page`), or as the raw answer the core keeps on disk and has `parse` read again (`Answer::Raw`, a Subsonic server's); what else an answer depends on (a server's music folders); which error means the item is gone |
| `write` | favorites, playlists and the rest of `Write`, where it has somewhere to send them |
| `stream`, `ahead`, `resolver` | where a song streams from and its cache key; how far ahead songs may be fetched; a `ResolveStream` that turns stream addresses into the URLs they open at, called by the engine's byte source (`nori_engine::core::resolving`), which reads a URL served as an HLS playlist as its segments (`crates/engine/src/hls.rs`) |
| `cover_address`, `cover_prefix` | cover URLs |
| `sign_in_done`, `sign_in`, `sign_out`, `account` | the account: when a sign-in page is done, signing in with what the page or the user handed over, and where it stands (`AccountState`) |
| `reminds_to_sign_in`, `stop_reminding` | whether the client reminds the user to sign in again after the session ended |
| `subsonic_api` | whether the core's Subsonic account work (scrobbles, playlists, queue pushes, remote control) has a server; without one those requests fail at once |

## What every provider gets from the core

A provider only fetches and sends. Everything else is the core's page store
(`crates/core/src/cache_policy.rs`), the same for every provider:

- Pages are kept under nori's own keys, each read with its freshness window. A kept page shows at once
  and is asked anew when stale. A page asked anew that has not changed is not shown again.
- The session's favorites are laid over every page as it is read: items unmarked this session leave the
  favorites, items marked join them before the source lists them.
- A write makes the pages of its kind stale (`cache_policy::stale`). They still show, offline too, until
  the source answers anew.
- Home stays as first shown for the whole session. A refresh merges the new feed into it, so nothing
  shown moves.

Tests:

- `providers/harness.rs` runs one scripted session per provider and compares each step's pages and
  requests with a recorded transcript in `crates/core/tests/golden/`. `NORI_BLESS=1` records it anew.
- `providers/fake.rs` is a test-only provider standing in for a third kind. Its tests search, open a
  page, play, show covers and sign in through the generic paths, with no provider named.

## What a client does

A client asks the core and draws:

| step | the core's calls |
|---|---|
| the first screen | a new profile starts here: `Kinds::providers`, in order, then quieter `nori_plugins::provider_offers`, what repos offer that isn't on. One not installed opens its plugin page, and once installed is turned on; one installed but off just turns on. Then `ProviderInfo::first_step` decides: a form to fill, titled with the provider's name; a sign-in page and nothing to fill (a name alone doesn't count), a page with its sign-in and Continue as guest; nothing at all, added at once, its page showing only if that fails. `ProviderInfo::after_adding` turns how it was added (`AfterAdding`) into what a client does next (`SignInStep`). Back from a page chooses again. With every plugin off (`pluginsOff`) no plugin is offered. Repos that can't be read leave `Kinds::providers` |
| the add form | the chosen kind's `ProviderInfo` fields, advanced ones folded away; `Kinds::server_ready` says whether it can be sent |
| logging in | `log_in`: `Kinds::login_profile`, then `Kinds::check_profile`; keeps the profile as the active one |
| opening a profile | `Core::configure_profile`; the engine's byte source wrapped by `nori_engine::core::resolving` |
| signing in | for a `SignIn::WebPage`, a page of the app's own at `start_url` whose addresses (each page, and each address it refuses to open: an OAuth redirect to an app's own address) and cookies go to `Kinds::sign_in_done` until it says done, then to `Client::sign_in`; for `SignIn::Paste`, a box to paste into. `Client::account` says where it stands |
| covers | `Core::cover_prefix` (the id and size appended), or `Core::cover_address` without one |

A client's only per-provider code is its words, one entry per kind:

| client | where |
|---|---|
| Android | `providerStrings` in `app/src/main/kotlin/dev/nori/music/app/ui/Say.kt` (string resources) |
| desktop | `words::provider` in `crates/desktop/src/words.rs` |
| terminal | `text::provider` in `crates/cli/src/text.rs` |
| iPod | `Say.provider` in `ios/Sources/Say.swift` |

## Adding a provider

A provider of a service nori doesn't ship is a plugin (its `provider` export; [Plugins](plugins.md)).
One built into nori:

1. Add a `ServerKind` variant in `crates/settings`, with the name its profiles are stored under
   (`ServerKind::id`, `ServerKind::from_id`).
2. Write its `Provider` as one module under `crates/core/src/providers/` and add its line to
   `Kinds::provider` and `Kinds::kinds`. Put a wire format in a crate of its own with no nori
   dependencies. Put it behind a nori-core feature when not every client builds it.
3. Add its words to each client's table above.

Nothing else changes: the forms, sign-in, pages, playback and rules follow from what it answers.
