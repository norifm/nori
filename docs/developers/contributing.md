# Contributing

Applies to the nori repository at 0.6.

Welcome. nori is MIT licensed, and changes come as pull requests on
[GitHub](https://github.com/norifm/nori).

## Before you start

- Read [AGENTS.md](../../AGENTS.md): it is the working agreement for people and agents alike, with
  the commands, the boundaries between the core and the clients, the code rules and the commit format.
- Build and test with [Building](building.md) and [Testing](testing.md). Run cargo with four jobs at most.
- A bug fix starts with a test that fails on the bug.

## Commits

One line, conventional, as [AGENTS.md](../../AGENTS.md#commits) describes. The changelog is made from
`feat`, `fix` and `perf` subjects, so write those for the people who use nori.

## The docs

- The user docs live in the website's repository,
  [norifm/norifm.com](https://github.com/norifm/norifm.com), under `src/content/docs/docs/`. A change
  that alters what users see can update them there.
- The developer docs are here, in `docs/developers/`, and change with the code in the same commit.
- Internal notes (`TODO.md`, `docs/research/`) stay unpublished.
- Every page says which versions and clients it applies to, and states only what the code does.

## Security

Report a vulnerability privately, as [SECURITY.md](../../SECURITY.md) says, not in a public issue.
