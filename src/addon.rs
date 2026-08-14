//! What an addon is: the trait an author implements, and its vocabulary.
//!
//! Moved here from `nobble-core`, where its own module doc already said these
//! types were "destined to be part of the public SDK (ADR-0010, FR-047)". They
//! arrived without a single `use crate::` line, so the move was a lift rather
//! than an extraction — the boundary was already where it needed to be.
//!
//! **Nothing here depends on any other Nobble crate, and it must stay that
//! way.** Every dependency added here is one a third-party addon author
//! inherits, and FR-044 says an addon must be buildable against only the public
//! repositories.

use core::fmt;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Where an addon keeps its own credentials (ADR-0013, FR-046).
///
/// # Why there is no `addon` parameter
///
/// Because there used to be, and it was a hole. `nobble-core`'s `SecretStore`
/// reads `fn get(&self, addon: &str, key: &str)`, and every addon was handed
/// the same live handle to it — so **any addon could read any other addon's
/// credentials** by passing a different string. `get("spotify",
/// "refresh_token")` from inside an unrelated addon returned the token.
///
/// That could not be fixed by narrowing the trait while addons shared an
/// address space: a handle to the store *is* the capability, and asking a third
/// party not to use one they hold is not a control. It is fixed by taking the
/// namespace out of the caller's hands entirely. An addon says which *key* it
/// wants and never which addon it is; whoever hands out the handle decides
/// that, and in the daemon it is bound to the addon the handle was made for.
///
/// The result is a type in which the old mistake cannot be written down.
/// [ADR-0016](../../../docs/decisions/0016-addon-process-boundary.md).
pub trait Credentials: Send + Sync {
    /// Read one of this addon's credentials.
    fn get(&self, key: &str) -> Option<String>;

    /// Store one.
    ///
    /// # Errors
    /// If the platform store refused it.
    fn set(&self, key: &str, value: &str) -> Result<(), String>;

    /// Forget one. Absent is the outcome, so forgetting nothing is success.
    fn clear(&self, key: &str);
}

/// A shared [`Credentials`], because an addon needs it after configuration too.
pub type CredentialHandle = Arc<dyn Credentials>;

/// Full scale for a 14-bit fader value.
///
/// Lives here rather than in `nobble-core` because [`Invocation::fraction`]
/// needs it and this crate may not depend on any other. `nobble-core`
/// re-exports it, so there is still one definition — the direction is decided
/// by the SDK's no-dependency rule rather than by where it feels like it
/// belongs.
///
/// 14-bit by ADR-0007 Amendment 1: 7 bits over a 100 mm throw is 0.78 mm per
/// step, which is felt as stepping and heard as zipper noise.
pub const FADER_MAX: u16 = 16_383;

/// What kind of input an action expects.
///
/// [ADR-0012]. Declared rather than inferred, because the two are not
/// interchangeable and binding one to the other is always a mistake: a fader
/// bound to "next track" would skip a hundred tracks across one sweep, and a
/// key bound to "set volume" has no position to send.
///
/// [ADR-0012]: ../../../docs/decisions/0012-addon-actions-carry-data.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// A press. The ordinary case, and the default an addon author should
    /// reach for.
    Momentary,
    /// A position — a fader, `0..=16383`.
    ///
    /// The daemon coalesces these, so an action sees where the fader ended up
    /// rather than every point it passed through. An action that needs the
    /// whole gesture is a different thing and this is not it.
    Continuous,
}

/// What a parameter holds, so an interface can offer the right editor.
///
/// A hint, not a storage type — every parameter is stored as a string
/// (ADR-0012). Widening this is additive; a client that does not recognise a
/// kind falls back to a text box and stays useful.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParamKind {
    /// Free text.
    Text,
    /// An application identity, in whatever form the foreground watcher
    /// reports. The interface can offer "the application you were just in"
    /// rather than asking someone to type an executable name from memory.
    App,
}

/// One thing an action needs to know, declared so an interface can ask for it.
///
/// The point of declaring rather than parsing a free-text argument: FR-048
/// wants addon configuration built from the shared component library, and there
/// has to be something to build it *from*. A third-party addon gets the same
/// editor as a first-party one with no code in the interface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddonParam {
    /// Stable identifier, and the key a binding stores it under. Renaming one
    /// silently drops whatever the user had chosen.
    pub id: &'static str,
    /// What to call it.
    pub name: &'static str,
    /// What it is for, in a sentence.
    pub description: &'static str,
    /// What it holds.
    pub kind: ParamKind,
    /// Whether the action can run without it.
    ///
    /// An optional parameter is a real thing rather than an oversight: the
    /// media addon's target application is absent for "whatever is playing",
    /// which is the behaviour most people want most of the time.
    pub required: bool,
}

/// One thing an addon can be asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddonAction {
    /// Stable identifier. This is what a binding stores, so it must not change
    /// once anyone could have saved it — a rename silently breaks their key.
    pub id: &'static str,
    /// What to call it in a menu.
    pub name: &'static str,
    /// What it does, in a sentence.
    pub description: &'static str,
    /// Whether it wants a press or a position.
    pub trigger: Trigger,
    /// What it needs to know, if anything.
    pub params: &'static [AddonParam],
}

impl AddonAction {
    /// One parameter's declaration, by id.
    #[must_use]
    pub fn param(&self, id: &str) -> Option<&'static AddonParam> {
        self.params.iter().find(|p| p.id == id)
    }
}

/// What an action was given when it ran.
///
/// Two sources, and keeping them apart is the whole design. **Parameters** were
/// chosen when the binding was authored and are saved with it; the **value**
/// was produced by the input a moment ago and is saved nowhere. Merging them
/// into one map would let a fader position be persisted, which is the sort of
/// thing that works until someone restarts the daemon and their volume jumps.
#[derive(Debug, Clone, Copy)]
pub struct Invocation<'a> {
    params: &'a BTreeMap<String, String>,
    value: Option<u16>,
}

/// Nothing configured, for an action that takes no parameters.
static NO_PARAMS: std::sync::LazyLock<BTreeMap<String, String>> =
    std::sync::LazyLock::new(BTreeMap::new);

impl<'a> Invocation<'a> {
    /// A press, with the binding's parameters.
    #[must_use]
    pub fn press(params: &'a BTreeMap<String, String>) -> Self {
        Self {
            params,
            value: None,
        }
    }

    /// A position, with the binding's parameters.
    #[must_use]
    pub fn moved(params: &'a BTreeMap<String, String>, value: u16) -> Self {
        Self {
            params,
            value: Some(value),
        }
    }

    /// A press with nothing configured. For tests, and for actions that take
    /// no parameters.
    #[must_use]
    pub fn bare() -> Self {
        Self {
            params: &NO_PARAMS,
            value: None,
        }
    }

    /// One parameter, if the binding set it.
    ///
    /// Absent and empty are the same answer here. A text box someone cleared
    /// stores `""`, and an addon checking only for absence would then treat a
    /// deliberately blank field as a configured one.
    #[must_use]
    pub fn param(&self, id: &str) -> Option<&str> {
        self.params
            .get(id)
            .map(String::as_str)
            .filter(|s| !s.is_empty())
    }

    /// Every parameter, for reporting.
    pub fn params(&self) -> impl Iterator<Item = (&str, &str)> {
        self.params.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// The raw position, `0..=16383`, for a continuous action.
    #[must_use]
    pub fn value(&self) -> Option<u16> {
        self.value
    }

    /// The position as a fraction of full travel, `0.0..=1.0`.
    ///
    /// Provided so the common case is one call rather than a division everyone
    /// writes slightly differently. **Not** a taper: audio wants a curve, and
    /// only the addon knows whether its target is already logarithmic.
    #[must_use]
    pub fn fraction(&self) -> Option<f32> {
        self.value.map(|v| f32::from(v) / f32::from(FADER_MAX))
    }
}

/// One thing an addon needs configuring once, rather than per key.
///
/// [ADR-0013]. A client id, an endpoint, a token. Declared with the same
/// vocabulary as an action's parameters so the interface renders a settings
/// page with no code per addon, which is the same FR-048 reasoning.
///
/// [ADR-0013]: ../../../docs/decisions/0013-addon-settings-and-secrets.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddonSetting {
    /// What to ask for.
    pub param: AddonParam,
    /// Whether this is a credential.
    ///
    /// A secret is kept in the OS credential store rather than in a file, and
    /// **is never sent back** — the interface can set one and can ask whether
    /// one exists, and cannot read it. FR-026 makes profiles shareable, and a
    /// refresh token that could be read back is one that ends up in a paste.
    pub secret: bool,
}

/// A named boolean fact an addon publishes about its target.
///
/// FR-062. Recording, streaming, in a call, muted, playing. An overlay can be
/// conditioned on one (ADR-0011), which makes a signal a **resolution input**
/// and therefore a stronger obligation than FR-047's display state: a stale
/// label is a cosmetic problem, a stale signal is a device doing the wrong
/// thing.
///
/// Boolean, and FR-063 keeps it that way until a specified need exists for
/// more. The same reasoning as the RPC being deliberately small: a value set
/// that grows on speculation grows a conversion for every consumer, and the
/// consumers here include third-party addons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddonSignal {
    /// Stable identifier, **unqualified** — `recording`, not `obs.recording`.
    /// The addon's own id qualifies it, so an addon cannot claim a name in
    /// another's space by choosing a clever string.
    ///
    /// Stored in configurations, so a rename silently breaks someone's
    /// overlay, exactly as [`AddonAction::id`] does.
    pub id: &'static str,
    /// What to call it.
    pub name: &'static str,
    /// What it means, in a sentence. Shown next to a live value, so it has to
    /// say what *true* means rather than what the signal is about: "Spotify is
    /// playing" beats "playback state".
    pub description: &'static str,
}

/// What an addon's signals read at one moment.
///
/// # Why the daemon asks rather than the addon telling
///
/// **Polled, not pushed**, and the reasons are all about what happens when
/// this interface crosses a process boundary — which FR-045 says it will.
///
/// A request/response call survives that move unchanged. A callback does not:
/// it needs a reverse channel, and a reverse channel is precisely where a
/// wedged addon becomes a wedged daemon, because something has to be waiting
/// on it.
///
/// Polling also puts the rate limit on the side that suffers from getting it
/// wrong. A pushing addon that reports a flapping signal thousands of times a
/// second is a churn source the daemon can only damp *after* paying for it,
/// and ADR-0011 is explicit that signal churn is flash wear rather than a slow
/// link. A polled addon cannot produce that by construction.
///
/// What polling costs is answered by the addon, not by the poll rate: an
/// implementation is free to keep a cache fed by platform events and answer
/// from it, which is what FR-065's "no measurable idle CPU" actually turns on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reading {
    /// It looked, and these are the values.
    ///
    /// Ids must be ones [`Addon::signals`] declares. Anything else is dropped
    /// by the daemon rather than becoming a signal nobody can find the
    /// definition of.
    Values(Vec<(&'static str, bool)>),
    /// It could not look, and this is why.
    ///
    /// FR-064: every signal it publishes then reads false, **and the reason is
    /// this sentence** rather than "the condition was false". The two states
    /// are indistinguishable to a user and have different fixes — one is a
    /// configuration mistake, the other is a closed application.
    Unavailable(String),
}

impl Reading {
    /// A reading of nothing, from an addon that publishes no signals.
    #[must_use]
    pub fn none() -> Self {
        Self::Values(Vec::new())
    }

    /// The value of one signal, if this reading carries it.
    #[must_use]
    pub fn get(&self, signal: &str) -> Option<bool> {
        match self {
            Self::Values(values) => values.iter().find(|(id, _)| *id == signal).map(|(_, v)| *v),
            Self::Unavailable(_) => None,
        }
    }
}

/// Whether an addon can act at the moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Availability {
    /// It can.
    Ready,
    /// It cannot, and this is why, in words a user can act on.
    ///
    /// "Spotify is not running" is useful. "Error 0x80070002" is not: a person
    /// reading it on a settings page cannot do anything with it.
    Unavailable(String),
}

/// Why performing an action failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddonError {
    /// No action by that name. Usually a binding saved against an older
    /// version of the addon.
    NoSuchAction(String),
    /// The addon could not act right now.
    Unavailable(String),
    /// It tried and something went wrong.
    Failed(String),
}

impl fmt::Display for AddonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSuchAction(a) => write!(f, "no action called {a:?}"),
            // Both render as their reason. The variants stay separate because
            // callers distinguish them -- unavailable is worth retrying, failed
            // is not -- but a reader wants the sentence, not the category.
            Self::Unavailable(why) | Self::Failed(why) => write!(f, "{why}"),
        }
    }
}

impl std::error::Error for AddonError {}

/// Something that can act on another application's behalf.
///
/// Implementations are expected to be cheap to construct and to discover their
/// target lazily: the daemon builds every addon at startup whether or not the
/// application it integrates is installed, and a constructor that blocked on a
/// network call would delay login for all of them.
pub trait Addon: Send {
    /// Stable identifier, stored in bindings. Never rename one.
    fn id(&self) -> &'static str;

    /// What to call it.
    fn name(&self) -> &'static str;

    /// What it integrates, in a sentence.
    fn description(&self) -> &'static str;

    /// Everything it can be asked to do.
    fn actions(&self) -> &'static [AddonAction];

    /// Whether it could act right now.
    ///
    /// Asked whenever the interface draws the addon, so it must be quick and
    /// must not block. It is allowed to be wrong a moment later — the target
    /// application can close between this answer and the next keypress, which
    /// is why [`Self::perform`] returns a result of its own rather than
    /// trusting this.
    fn availability(&self) -> Availability;

    /// Do it.
    ///
    /// `invocation` carries the binding's parameters and, for a
    /// [`Trigger::Continuous`] action, the position the input reported.
    ///
    /// # Errors
    /// If the action is unknown, a required parameter is missing, the target is
    /// unavailable, or it failed.
    fn perform(&mut self, action: &str, invocation: &Invocation<'_>) -> Result<(), AddonError>;

    /// Whether an action is worth offering at the moment.
    ///
    /// For the pair that undo each other — sign in and sign out — where
    /// offering both at once means one of them is always the wrong half of a
    /// question nobody asked.
    ///
    /// # This is relevance, not permission
    ///
    /// [`Self::perform`] deliberately does **not** consult it, and neither
    /// does [`Self::actions`], which stays complete. Three consequences, all
    /// intended:
    ///
    /// - A key bound to a hidden action still works. Bindings are authored
    ///   once and pressed later, and a key that stopped existing because of
    ///   something that happened after it was bound is the failure US9 calls
    ///   out.
    /// - The action can still be *bound*, because the editor is about what a
    ///   key could ever do rather than what is useful this second.
    /// - An addon must still handle being asked. Signing out twice succeeds;
    ///   this only stops the interface suggesting it.
    ///
    /// Defaulted to `true`, which is the ordinary case: an action that is
    /// worth having is worth having now.
    fn applies(&self, _action: &str) -> bool {
        true
    }

    /// One line about what it is currently connected to, when that is a
    /// question a user can have.
    ///
    /// Shown wherever the addon is drawn, next to its name.
    ///
    /// # Why this is not part of [`Availability`]
    ///
    /// They answer different questions, and only one of them has an answer
    /// when things are fine. `Availability` is *can it act*, and the useful
    /// case is the negative one — an addon that cannot act owes the user a
    /// reason. `Ready` carries no words because "it works" needs none.
    ///
    /// This is *what is it working as*, and it is only interesting when the
    /// answer is **ready**. "Signed in as Philipp" tells someone which of two
    /// Spotify accounts their Save key is filling up, which is invisible from
    /// anywhere else in the interface and is exactly the thing they need before
    /// pressing Sign out. Folding it into `Ready(String)` would have made every
    /// addon that has nothing to say construct an empty one.
    ///
    /// Defaulted to `None`, which is the ordinary case: an addon that talks to
    /// a local application is connected to the only thing it could be.
    ///
    /// Called whenever the interface draws the addon, so it must be quick and
    /// must not block — the same contract as [`Self::availability`], and the
    /// same reason. Read it from what the addon already holds; do not go and
    /// ask the network.
    fn status(&self) -> Option<String> {
        None
    }

    /// Every signal it publishes. FR-062.
    ///
    /// Defaulted, because most addons only act. An addon that publishes none
    /// is not a lesser addon — it is the ordinary case, and the interface
    /// should not make it write an empty slice to say so.
    fn signals(&self) -> &'static [AddonSignal] {
        &[]
    }

    /// Everything it needs configuring once (ADR-0013).
    ///
    /// Defaulted for the same reason as signals: most addons need nothing, and
    /// the ones that do are the exception.
    fn settings(&self) -> &'static [AddonSetting] {
        &[]
    }

    /// Take its settings, and a place to keep its own credentials.
    ///
    /// Called at startup and whenever the settings change, so an addon holds
    /// what it needs rather than asking. [`Credentials`] arrives as a handle
    /// rather than a value because an addon needs it *later* too — a refreshed
    /// token has to go back, and that happens mid-action rather than at
    /// configuration time.
    fn configure(&mut self, _values: &BTreeMap<String, String>, _credentials: CredentialHandle) {}

    /// Read every signal it publishes, now.
    ///
    /// Called on the signal poll, **never on the input path** — the same rule
    /// as [`Self::perform`], and for a stronger reason: this runs on a timer,
    /// so an implementation that blocked for a second would do it forever
    /// rather than once per keypress.
    ///
    /// See [`Reading`] for why this is a question the daemon asks rather than
    /// something the addon announces.
    fn read_signals(&mut self) -> Reading {
        Reading::none()
    }
}
