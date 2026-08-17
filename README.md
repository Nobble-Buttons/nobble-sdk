# Nobble addon SDK

Write an addon for [Nobble](https://github.com/Nobble-Buttons), a modular macro keyboard whose keys
carry their own displays and change with the application you are using.

**API documentation:** [nobble-buttons.github.io/nobble-sdk](https://nobble-buttons.github.io/nobble-sdk/)
— rustdoc for all three crates, rebuilt from `main` on every push.

Licensed under **Apache-2.0** — see [LICENSE](LICENSE) and [NOTICE](NOTICE).

**Permissive on purpose.** The desktop client is source-available under noncommercial terms; this is
not, and the difference is deliberate. A restrictive licence on an SDK propagates into everything
written with it, so an addon you write is yours: closed-source if you like, sold if you like. That is
the point of the boundary, not an oversight in it.

Apache-2.0 grants no rights in the Nobble marks (§6). See [TRADEMARKS.md](TRADEMARKS.md).

## What an addon is

A separate program. The daemon starts it, speaks line-delimited JSON to its stdin and stdout, and
supervises it — a crash is an exit code, a hang is a timeout, a leak is capped and killed. Nothing an
addon does can take the daemon down with it, which is the whole reason for the process boundary.

You implement one trait and call `run()`. The SDK owns the loop, the framing and the protocol
version; you never see a frame.

```rust
use nobble_addon_sdk::{Addon, AddonAction, AddonError, Invocation, Trigger};

struct Hello;

impl Addon for Hello {
    fn id(&self) -> &'static str { "hello" }
    fn name(&self) -> &'static str { "Hello" }
    fn description(&self) -> &'static str { "Prints when you press a key" }

    fn actions(&self) -> &'static [AddonAction] {
        const ACTIONS: &[AddonAction] = &[AddonAction {
            id: "greet",
            name: "Greet",
            description: "Say hello",
            trigger: Trigger::Momentary,
            params: &[],
        }];
        ACTIONS
    }

    fn perform(&mut self, action: &str, _invocation: &Invocation<'_>) -> Result<(), AddonError> {
        if action != "greet" {
            return Err(AddonError::NoSuchAction(action.to_owned()));
        }
        eprintln!("hello");
        Ok(())
    }
}

fn main() { nobble_addon_sdk::run(Hello); }
```

Build it as `nobble-addon-hello` and put the binary beside the daemon. **The daemon takes an addon's
identity from the filename, not from what the program says about itself** — it has to know whose
credentials it is holding before it trusts anything the child sends.

## What is in here

| Crate | What it is |
|---|---|
| `nobble-addon-sdk` | the SDK. Depends on `serde` and `serde_json` and nothing else. |
| `nobble-addon-media` | what is playing, from the OS. Uses WinRT. |
| `nobble-addon-volume` | system volume, per application. Uses Core Audio. |

Both addons ship with the product **and get no shortcut**. They are built against this SDK and
nothing else of ours, which is a rule with teeth: there is no other Nobble crate in this repository
to reach for, and CI asserts the dependency graph rather than trusting a review. If an official addon
ever needs an interface you cannot have, the boundary is in the wrong place, and that is a defect
rather than a licence to add one.

`volume` is the smaller and the better one to read first.

### Why there is no Spotify or Discord addon here

Both exist and ship with the product; they live in the daemon's repository instead. Addons that
integrate a **third-party service** are deliberately kept out of this one:

- a partner's developer terms travel with the addon, and a public repository gets forked, which
  multiplies whatever those terms constrain;
- the Spotify addon carries a shared client id that every fork would spend, because the quota is
  metered per application rather than per user;
- a partner addon is the one that grows daemon-side tooling. The Spotify addon's example signs in
  against a real account and writes to the daemon's own credential vault, so it depends on three
  private crates and could not build here at all.

None of that limits what **you** may integrate. The SDK does not know what a partner is; the rule is
about which examples we publish, not about what an addon is allowed to do.

## Permissions

An addon declares what it needs and the user grants it explicitly. Credentials are **enforced**: the
daemon holds the store and hands over a secret only for an addon that declared and was granted
`Permission::Credentials`. You cannot ask on another addon's behalf, because you never say who you
are — the daemon knows that from the pipe.

Network hosts, filesystem paths and process launch are **declared and shown, not confined**. A
granted addon can still open a socket nobody sanctioned. The interface says so in those words rather
than in a footnote, because a permission list that reads as authoritative is worse than none.

## Building

```bash
cargo build --workspace
cargo test --workspace
```

Nothing here needs the daemon, the hardware, or any private repository. If a build of this
repository alone ever stops working, that is a defect in the boundary and not in your checkout.

## Versioning

The protocol carries its own version. The daemon refuses an addon built against an incompatible one,
naming the fix, rather than loading it and failing strangely later. This repository versions
independently of the daemon — that is what splitting it out was for.
