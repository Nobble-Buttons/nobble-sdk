# Nobble addon SDK — agent context

The public, permissive surface of the project: the addon SDK plus the official addons that integrate
nothing but the operating system. [`README.md`](README.md) is written for the people who will build
against this; read it first, because everything below assumes it.

**This is the most exposed repository in the project.** It is Apache-2.0, it is meant to be forked,
and its API is a promise to strangers. Treat every commit here as permanent and public — because it
is both.

## What may live here

Two rules, both already stated in [`Cargo.toml`](Cargo.toml) and both load-bearing.

**Nothing may depend on a Nobble crate other than `nobble-addon-sdk`.** That is what makes the
addons here *worked examples* rather than privileged ones — an addon you write has exactly the
access ours do. CI checks it; the layout also enforces it, since there is no private crate within
reach.

**No addon that integrates a third party belongs here.** `media` and `volume` ask the operating
system what is playing and how loud it is. Anything that talks to a partner service stays in the
daemon's repository: a partner's developer terms travel with the addon, a public repository gets
forked, and a shared application credential is spent by every fork that inherits it.

## Licence direction

**Apache-2.0, and dependencies flow one way.** This repository may not depend on anything under
PolyForm Noncommercial — the desktop client depends on the SDK, never the reverse. A restriction
reached through an SDK propagates into everything third parties write with it, which is precisely
the outcome the split exists to prevent. Adding a copyleft dependency breaks the same promise from
the other side.

Apache-2.0 grants no rights in the Nobble marks (§6) — see [TRADEMARKS.md](TRADEMARKS.md). Keep
[NOTICE](NOTICE) current when files gain or change attribution.

## Working here

**Nothing private enters this repository.** No hardware detail, no part numbers, no partner
credentials or terms, no unpublished specification text — in code, comments, tests, or commit
messages. The parent project is private; this is not; and a commit cannot be unpublished.

**This is a workspace of its own, and building alone is the acceptance test.** It is `exclude`d from
the daemon's workspace deliberately. If anything here stops compiling without the daemon present,
the split has been broken — that is the failure, not the build error.

```bash
export PATH="$PATH:$HOME/.cargo/bin"   # cargo is installed but not on PATH
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

**An API change here is a breaking change for people you cannot contact.** Additive by default;
where a break is genuinely necessary, say so in the commit subject and in the crate's changelog
rather than letting a minor version carry it quietly.

## Committing

Subjects say what was learned or what it cost, not what was edited. Sentence case, no Conventional
Commits, no `feat(scope):` prefixes, no trailing period.

**Stage paths, never `git add -A`** — sessions run concurrently in this tree and share one working
directory.

**This repository is the deepest link in a three-level submodule chain**: here → the desktop client →
the private parent. Push here *first*; a pointer to an unpushed commit exists nowhere but this disk
and kills a recursive clone two levels up.
