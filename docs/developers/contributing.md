# Contributing

You contribute to nori with a pull request on [GitHub](https://github.com/norifm/nori). nori is
licensed AGPL-3.0-or-later ([LICENSE](../../LICENSE)).

## Before you start

- Read [AGENTS.md](../../AGENTS.md). It is the working agreement for people and agents alike: the
  commands, what belongs in the core and what in a client, and the rules for code and tests.
- Read [Architecture](architecture.md) for the layers and the rules that hold.
- Set up your machine with [Building](building.md), and learn the test tiers in [Testing](testing.md).
- Run Cargo with at most four jobs (`-j4`). More jobs can run the machine out of memory.

Two rules shape most changes:

- **A bug fix starts with a test that fails on the bug.** Find the cause first, then fix it there.
- **A feature is built in Rust first.** Then it is wired into every client that has the screen for it:
  Android, the desktop app, the terminal app, the iPod app and the web client. If you leave a client
  out, say which one and why in the pull request.

## Before you commit

Before each commit, run the tests and clippy of the crates that your change reaches:

```sh
tools/check.sh changed
```

Before a batch of commits goes to master, or before you mark a pull request ready, run the checks that
the batch reaches:

| The batch changes | Run |
|---|---|
| Anything | `tools/check.sh rust --workspace` and `tools/check.sh clippy --workspace` |
| A crate the web or iPod clients use (core, host, engine, remote) | `tools/check.sh web` and `tools/check.sh ipod` |
| Code that reaches Android | `tools/check.sh android`, `tools/smoke.sh`, and the `--only` sections of `tools/audio-e2e.sh` and `tools/feature-e2e.sh` that it touches |

`tools/check.sh` runs one heavy build at a time across worktrees. Two at once make the machine swap.

CI fails on any clippy warning. `tools/audio-e2e.sh --list` and `tools/feature-e2e.sh --list` name the
sections. [Testing](testing.md) explains the device checks and the emulator they need.

## Commits

A commit message is one line, in the conventional form, lowercase, with no full stop and under 72
characters:

```text
<type>: <what is different now>
```

The type is one of `feat`, `fix`, `perf`, `refactor`, `docs`, `build`, `test` or `chore`. A commit has no
body and no trailers. The changelog is made from the `feat`, `fix` and `perf` subjects, so write those
for the people who use nori: `fix: bluetooth device retries no longer overlap`.

One piece of work is one commit. If you fix your own work before it is merged, amend the commit it
fixes instead of adding a new one.

## Pull requests

A pull request lands on `master` as one squashed commit, after the owner approves the merge. The
commit's subject is the pull request's title and number, `feat: <what> (#81)`, so write the title in
the commit form above.

The description names every issue the pull request settles, one per line: `Closes #12` or `Fixes #34`,
and `Refs #56` for an issue it only helps. GitHub then links and closes them.

## Docs

The docs for a change go in one of three places:

| Docs | Where | When they change |
|---|---|---|
| Developer docs | `docs/developers/` in this repository | With the code, in the same commit |
| User docs | `src/content/docs/docs/` in `norifm/norifm.com` (private) | When a change alters what users see |
| Internal notes | `TODO.md` and `docs/research/` | Never published |

The writing rules for every page are in `.claude/skills/writing-docs/SKILL.md`. Its lint checks a page:

```sh
python3 .claude/skills/writing-docs/scripts/docs-lint.py docs/developers/<page>.md
```

## Security

Report a vulnerability privately, as [SECURITY.md](../../SECURITY.md) says. Do not open a public issue
for it.
