# Contributing

Applies to nori 0.7, every client.

nori is licensed AGPL-3.0-or-later ([LICENSE](../../LICENSE)). Changes come as pull requests on
[GitHub](https://github.com/norifm/nori).

## Before you start

- Read [AGENTS.md](../../AGENTS.md). It is the working agreement for people and agents alike: the
  commands, what belongs in the core and what in a client, the code rules and the commit format.
- Build with [Building](building.md) and test with [Testing](testing.md). Run Cargo with at most four
  jobs (`-j4`).
- A bug fix starts with a test that fails on the bug.
- A feature is built in Rust first, then wired into every client that has the screen for it.

## Before committing

| change | run |
|---|---|
| any | `cargo test -j4 --workspace` and a build |
| reaches Android | also `tools/smoke.sh` and the `tools/audio-e2e.sh` / `tools/feature-e2e.sh` sections it touches (`--only`) |
| Rust | `cargo clippy -j4 --workspace --all-targets`, no new warnings |

## Commits

One line, conventional, as [AGENTS.md](../../AGENTS.md#commits) describes:
`<type>: <what is different now>`, no body and no trailers. The changelog is made from `feat`, `fix`
and `perf` subjects, so write those for the people who use nori.

## Docs

| docs | where | changes |
|---|---|---|
| user docs | `norifm/norifm.com` (private), `src/content/docs/docs/` | when a change alters what users see |
| developer docs | `docs/developers/` here | with the code, in the same commit |
| internal notes | `TODO.md`, `docs/research/` | never published |

Every page says which versions and clients it applies to, and states only what the code does.

## Security

Report a vulnerability privately, as [SECURITY.md](../../SECURITY.md) says, not in a public issue.
