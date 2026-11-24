# nori developer docs

These pages explain how nori is built, for people who change its code or write plugins for it. They
describe the code on `master`. The user docs, for listeners, are at
[norifm.com/docs](https://norifm.com/docs/), from `src/content/docs/docs/` in the website's repository,
`norifm/norifm.com`.

[AGENTS.md](../../AGENTS.md) is the working agreement for people and agents alike: the commands, what
belongs in the core and what in a client, and the rules for code, tests and commits. Read it with
these pages.

## Where to start

- **Fix a bug or add a feature.** Read [Architecture](architecture.md), then [Building](building.md)
  and [Testing](testing.md). [Contributing](contributing.md) says how a change lands.
- **Write or port a client.** Read [Architecture](architecture.md), then
  [Writing a client](writing-a-client.md) and [Providers](providers.md).
- **Write a plugin.** Start with [Plugins](plugins.md), build one with the
  [plugin tutorial](plugin-tutorial.md), and look things up in the [plugin reference](plugin-reference.md).
- **Change the plugin host.** Read [The plugin host](plugin-host.md).
- **Make nori faster or lighter.** Read [Performance](perf.md). For the desktop app, also read
  [Desktop profiling](desktop-performance.md).

## The pages

| Page | For | What it holds |
|---|---|---|
| [Architecture](architecture.md) | Every contributor | The layers, the crates, one song traced end to end, the rules that hold |
| [Building](building.md) | Every contributor | Setup, builds for each platform, the perf build, releases |
| [Testing](testing.md) | Every contributor | The test tiers, the local server, device checks |
| [Contributing](contributing.md) | Every contributor | The checks before a commit, commits, pull requests, where docs go |
| [Writing a client](writing-a-client.md) | Client authors | What the core does, what a client builds, how to call the core |
| [Providers](providers.md) | Client and core authors | Where a profile's music comes from, and how to add a source |
| [Pairing and profile sync](sync.md) | Client and core authors | The group key, pairing a device, and how profiles merge |
| [Plugins](plugins.md) | Plugin authors | What a plugin can do, and how it runs |
| [Plugin tutorial](plugin-tutorial.md) | Plugin authors | A first plugin, built step by step |
| [Plugin reference](plugin-reference.md) | Plugin authors | The manifest, the API, the extension points |
| [The plugin host](plugin-host.md) | Core authors | How nori installs, compiles and runs plugins |
| [Performance](perf.md) | Performance work | How nori is measured, and the benches |
| [Desktop profiling](desktop-performance.md) | Performance work | Profiling the desktop app on Linux and macOS |
| [iPod internals](ipod-internals.md) | iPod work | The iPod app: its build, its audio, its limits |
| [Supporting nori](support.md) | Support page work | The Support page and its payment service |

`TODO.md` lists ideas for later, and `docs/research/` holds research and plans. Neither is published.
