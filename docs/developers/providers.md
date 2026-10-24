# Providers

Applies to the nori repository at 0.6, every client.

A profile plays from a **provider**: a Subsonic server (Navidrome, octo-fiesta) or YouTube Music today.
Each provider is one implementation of the `Provider` trait in `crates/core/src/providers/`. Nothing
else in the core, the engine or a client names one: they ask the profile's provider, and draw what it
describes. (Not to be confused with octo-fiesta's provider items, the `ext-` songs a Subsonic server
fetches from elsewhere.)

## What a provider answers

`nori_core::providers::Provider`, one instance per profile opened (`Core::configure_profile`), so it can
hold that profile's state (a session, streams resolved):

| method | what |
|---|---|
| `info` | the add form's fields (`FormField`: name, address, user, ...; plain or advanced) and how its account signs in (`SignIn::WebPage { start_url }`, `SignIn::Paste`) |
| `ready`, `check` | whether the form can be sent; checking a profile before it is kept (a server is pinged) |
| `rules`, `configure` | the profile's `ProfileRules` (library sections, hearts, remote control, jams, downloads, its own Home feed...) and pointing the `Core` at it |
| `read`, `parse`, `scope`, `gone` | the core's `Read`s (search, album, artist, playlist, Home feed, library sections) as `Page`s (`Answer::Page`), or as the raw answer the core keeps on disk and has `parse` read again (`Answer::Raw`, a Subsonic server's); what else an answer depends on (a server's music folders); which error says the item is gone |
| `write` | hearts, playlists and the rest of `Write`, where it has somewhere to send them |
| `stream`, `ahead`, `resolver` | where a song streams from and its cache key; how far ahead songs may be fetched; a `ResolveStream` that turns its stream addresses into the URLs they open at, which the engine's byte source calls (`nori_engine::core::resolving`) |
| `cover_address`, `cover_prefix` | cover URLs |
| `sign_in_done`, `sign_in`, `sign_out`, `account` | the account: when a sign-in page is done, signing in with what the page or the user handed over, and where it stands |
| `subsonic_api` | whether the core's Subsonic account work (scrobbles, queue pushes, remote control) has a server; without one those requests fail at once |

## What every provider gets from the core

A provider only fetches and sends. The rest is the core's page store (`crates/core/src/cache_policy.rs`),
the same for every provider, so one cannot behave differently from another:

- pages kept under nori's own keys, each read with its freshness window, shown at once and asked anew
  when stale; a page asked anew that is the same is not shown again;
- the session's hearts laid over every page as it is read: the favourites lose what was unhearted and
  gain what was hearted, ahead of the source listing it;
- writes make the pages of their kind stale (`cache_policy::stale`): still shown, offline too, until the
  source answers anew;
- Home kept as first shown for the whole session; a refresh merges the new feed into it, so nothing
  shown moves.

`providers/harness.rs` runs one scripted session per provider and compares each step's pages and
requests with a recorded transcript (`crates/core/tests/golden/`); `NORI_BLESS=1` records it anew.

`providers/fake.rs` is a test-only provider standing in for a third kind: its tests search, open a
page, play, show covers and sign in through the generic paths, with no provider named.

## What a client does

A client asks the core and draws:

- the kinds offered: `providers()`, in order; a client offers the choice only when there are several;
- the add form: the fields of the chosen kind's `ProviderInfo`, advanced ones folded away; whether it
  can be sent (`server_ready`), and the login itself, which checks the profile and keeps it as the active
  one (`log_in`: `server_for_login`, then `check_profile`);
- opening a profile: `Core::configure_profile`, and the engine's byte source wrapped by
  `nori_engine::core::resolving`;
- signing in, for a kind with `sign_in` ways: a page of the app's own at `start_url`, whose address and
  cookies go to `sign_in_done` until it says done, then to `Client::sign_in`; or a box to paste into.
  `Client::account` says where it stands;
- covers: `Core::cover_prefix` (the id and size appended) or, without one, `Core::cover_address`.

The only per-provider code a client has is its words, one entry per kind:

| client | where |
|---|---|
| Android | `providerStrings` in `app/.../ui/Say.kt` (string resources) |
| desktop | `words::provider` in `crates/desktop/src/words.rs` |
| terminal | `text::provider` in `crates/cli/src/text.rs` |
| iPod | `Say.provider` in `ios/Sources/Say.swift` |

## Adding a provider

1. A variant of `ServerKind` (`crates/settings`) and the name its profiles are stored under
   (`ServerKind::id`, `ServerKind::ALL`).
2. Its `Provider` implementation, one module under `crates/core/src/providers/` (the wire format, if
   any, in a crate of its own with no nori dependencies, as innertube-music is, in its own repository), behind a feature of
   nori-core when not every client builds it, and its line in `providers::provider`.
3. Its words in each client's table above.

Nothing else changes: the forms, sign-in, pages, playing and the rules follow from what it answers.
