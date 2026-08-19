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

use serde::{Deserialize, Serialize};

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

impl Trigger {
    /// How it is spelled on a wire.
    ///
    /// # Why this is a method and not a `match` at each end
    ///
    /// Because it was three `match`es, and they had to agree without anything
    /// making them. The addon-to-daemon encoder lived in
    /// [`run`](crate::run), the daemon-to-interface encoder in
    /// `nobble_rpc::AddonDto::of`, and the decoder between them was
    /// `if a.trigger == "continuous"` in the daemon — with a TypeScript
    /// `=== "continuous"` at the far end comparing against the result. Five
    /// hand-written copies of two strings, on a round trip where a single
    /// disagreement makes a fader silently behave like a key: the decoder's
    /// `else` branch is `Momentary`, so a renamed encoding does not fail, it
    /// degrades.
    ///
    /// Constitution VI is about exactly that, and the remedy it prefers —
    /// generated bindings — is unavailable inside one language. One definition
    /// on the type is the next thing: `nobble-core` re-exports this type, so
    /// every crate downstream is looking at the same `impl` rather than at its
    /// own copy of the answer.
    ///
    /// **Not a display name.** These are protocol tokens, lowercase because
    /// that is what is on the wire, and a renaming here is a breaking protocol
    /// change rather than a wording change. An interface wanting *Momentary*
    /// with a capital M writes that itself.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Momentary => "momentary",
            Self::Continuous => "continuous",
        }
    }

    /// Read one back, or `None` if this build has no name for it.
    ///
    /// An `Option` rather than a default, deliberately. The daemon's decode
    /// used to fall back to [`Self::Momentary`] for anything unrecognised,
    /// which turns an addon built against a newer SDK into a fader that behaves
    /// like a key with nothing reported — a Principle IV collapse. Whether to
    /// refuse or to default is the caller's decision to state out loud; this
    /// only declines to make it for them.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "momentary" => Some(Self::Momentary),
            "continuous" => Some(Self::Continuous),
            _ => None,
        }
    }
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

impl ParamKind {
    /// How it is spelled on a wire. See [`Trigger::as_wire`] for why this is
    /// here rather than at each end.
    ///
    /// **Exhaustive, and that is the point of putting it in this crate.** This
    /// type is `#[non_exhaustive]`, so every match on it *outside* the SDK must
    /// carry a wildcard — which is how `nobble_rpc::param_kind_name` came to
    /// have a `ParamKind::Text | _ => "text"` arm defending against a variant
    /// that cannot exist in a build that compiled. Here the wildcard is not
    /// required, so adding a kind fails to compile at the one site that has to
    /// decide what it is called.
    #[must_use]
    pub const fn as_wire(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::App => "app",
        }
    }

    /// Read one back, or `None` if this build has no name for it.
    ///
    /// Unlike [`Trigger::from_wire`], defaulting is the *documented* behaviour
    /// for a caller here: this type's own note says widening it is additive and
    /// a client that does not recognise a kind should offer a text box and stay
    /// useful. The `Option` is still the honest return, because "I do not know
    /// this one" and "this one is text" are different facts and only the caller
    /// knows whether the difference matters to it.
    #[must_use]
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "text" => Some(Self::Text),
            "app" => Some(Self::App),
            _ => None,
        }
    }
}

/// Whether something the platform named is the application a binding meant.
///
/// Here rather than in each addon, because [`ParamKind::App`] is declared here
/// and an addon author has no way to guess how to compare one. The value
/// stored in a binding is whatever Nobble's foreground watcher reports —
/// `exe:spotify.exe` — and what the addon is comparing it against comes from
/// somewhere else entirely, in whatever form *that* API uses.
///
/// So the comparison is on the **stem**: `exe:spotify.exe` and `Spotify.exe`
/// both reduce to `spotify`. The Windows media session reports `Spotify.exe`
/// for a desktop install and `SpotifyAB.SpotifyMusic_…!Spotify` for the Store
/// build, and matching either exactly would work on one machine and not the
/// next.
///
/// An empty target matches **nothing**, which is the opposite of what an
/// unconstrained match would do. Naming a target means "this one", and a
/// blank field that matched everything would turn a typo into a key that
/// controls whatever happens to be loudest.
///
/// ```
/// # use nobble_addon_sdk::app_matches;
/// assert!(app_matches("Spotify.exe", "exe:spotify.exe"));
/// assert!(app_matches("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify", "exe:spotify.exe"));
/// assert!(!app_matches("chrome.exe", "exe:spotify.exe"));
/// assert!(!app_matches("Spotify.exe", ""));
/// ```
#[must_use]
pub fn app_matches(reported: &str, target: &str) -> bool {
    let stem = target
        .rsplit(':')
        .next()
        .unwrap_or(target)
        .trim_end_matches(".exe")
        .trim_end_matches(".EXE")
        .to_ascii_lowercase();
    // `Invocation::param` already treats a cleared field as absent, so reaching
    // here with an empty target is a bug rather than a user choice.
    !stem.is_empty() && reported.to_ascii_lowercase().contains(&stem)
}

/// What one parameter holds: a value, or several.
///
/// [ADR-0022]. A parameter is single-valued unless its declaration says
/// otherwise, and both shapes live in the same map because a binding stores one
/// map whatever its parameters declared.
///
/// # Why an enum rather than always a list
///
/// Because of what it costs on disk. `#[serde(untagged)]` renders these as
/// `app = "spotify"` and `apps = ["discord", "game"]` in the same TOML table,
/// so a configuration written before lists existed parses unchanged and needs
/// no migration rung. Making every value a list would have rewritten every
/// binding anybody has, to express something almost none of them use.
///
/// It is also what `006-FR-024a-i` asks for in the negative: a list "MUST be
/// widened rather than worked around with a delimiter convention the interface
/// cannot render". A TOML array is how TOML writes a list. There is no
/// convention to learn, and nothing to escape.
///
/// [ADR-0022]: ../../../docs/decisions/0022-addon-supplied-choices.md
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// One value. What every parameter was before ADR-0022.
    One(String),
    /// Several, in the order the user arranged them.
    Many(Vec<String>),
}

impl ParamValue {
    /// The single value, or `None` if this holds a list.
    ///
    /// **Not the first element**, deliberately. An addon that declared a
    /// single-valued parameter and receives a list has been given something it
    /// did not ask for, and quietly using the first entry would silently ignore
    /// the rest — the failure being a key that mutes one of the three
    /// applications somebody named. `None` reaches the required-parameter check
    /// in `Registry::perform` and fails loudly instead.
    #[must_use]
    pub fn one(&self) -> Option<&str> {
        match self {
            Self::One(v) if !v.is_empty() => Some(v),
            Self::One(_) | Self::Many(_) => None,
        }
    }

    /// Every value, whether this holds one or several.
    ///
    /// A single value reads as a list of one, because an addon that declared
    /// `multiple` should not have to care how the user happened to fill it in —
    /// and a file written before lists existed contains exactly that case.
    /// Empty strings are dropped for the same reason [`Self::one`] rejects
    /// them: a cleared box is not a configured value.
    #[must_use]
    pub fn all(&self) -> Vec<&str> {
        match self {
            Self::One(v) => {
                if v.is_empty() {
                    Vec::new()
                } else {
                    vec![v.as_str()]
                }
            }
            Self::Many(vs) => vs
                .iter()
                .map(String::as_str)
                .filter(|s| !s.is_empty())
                .collect(),
        }
    }
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        Self::One(v.to_owned())
    }
}

/// One option a user can pick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddonChoice {
    /// What gets stored in the binding.
    pub value: &'static str,
    /// What the user reads.
    pub label: &'static str,
}

/// One option from a [live](AddonChoices::live) source.
///
/// The owned twin of [`AddonChoice`], and it has to be owned: a roster is
/// people who happen to be in a call right now, so there is nothing to borrow
/// from and nothing `'static` to point at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// What gets stored in the binding.
    ///
    /// **An identity, not a name.** Whatever survives the label changing —
    /// a rename, a nickname, a rejoin — because this is what the binding still
    /// holds next week.
    pub value: String,
    /// What the user reads while choosing.
    pub label: String,
    /// A second line, where the label alone is ambiguous.
    ///
    /// Two people called Alex in one call is ordinary, and the identity
    /// underneath is a number nobody recognises. Empty when there is nothing to
    /// add, which is the common case.
    pub detail: String,
}

impl Choice {
    /// One with nothing to disambiguate it.
    #[must_use]
    pub fn new(value: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: String::new(),
        }
    }

    /// One that needs a second line to tell it from another.
    #[must_use]
    pub fn detailed(
        value: impl Into<String>,
        label: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            detail: detail.into(),
        }
    }
}

/// A named list of options an addon offers, which a parameter can draw from.
///
/// [ADR-0022]. **Named rather than attached to one parameter**, and the third
/// reason below is the one that decided it:
///
/// - a live fetch is then one addon and one id, where a parameter would need
///   `(addon, action, param)` — and a *setting*'s parameter has no action;
/// - two parameters can share one list, so a pin and a priority pointing at the
///   same people cannot show two different rosters;
/// - **composition belongs to the addon.** `006-FR-011b` wants the current call,
///   then people remembered from previous calls, then free text — an ordering
///   that is Discord's business. With a named source the addon returns one flat
///   ordered list and the interface renders it, so "remembered people" needs no
///   interface support at all.
///
/// [ADR-0022]: ../../../docs/decisions/0022-addon-supplied-choices.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddonChoices {
    /// Stable id, unique within the addon. A parameter names this.
    pub id: &'static str,
    /// What the list is, for the editor's heading and its empty state.
    pub name: &'static str,
    /// Whether the values must be asked for rather than read from below.
    ///
    /// A **declared** source is fixed for the life of the daemon and arrives
    /// with everything else. A **live** one changes — a voice roster, the
    /// playlists on an account — and is fetched when somebody opens the picker
    /// and at no other time. Not polled: a timer would cost idle CPU for a
    /// picker nobody has open, which `006-FR-027` forbids and `006-SC-008`
    /// measures.
    pub live: bool,
    /// Every value, for a declared source. Empty when [`Self::live`].
    pub values: &'static [AddonChoice],
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
    /// Whether it holds several values rather than one (`006-FR-024a-i`).
    ///
    /// Orthogonal to [`Self::kind`] on purpose. What a value *means* and how
    /// many of them there are are different questions, and folding one into the
    /// other is what produces `AppList`, `ParticipantList`, `PlaylistList` —
    /// one new kind every time somebody wants a list of something.
    pub multiple: bool,
    /// The id of an [`AddonChoices`] this draws its options from, if any.
    ///
    /// `None` is free text, which is what every parameter was before ADR-0022
    /// and what a choice-backed one degrades to when its list is unavailable.
    pub choices: Option<&'static str>,
}

impl AddonParam {
    /// A parameter with nothing set, to build from.
    ///
    /// Exists so the next field added here does not break every `const` array
    /// an addon writes: `AddonParam { id: "app", ..AddonParam::BASE }` compiles
    /// in a `const`, and adding a field to `BASE` costs its authors nothing.
    /// **It does not save the arrays that already name every field**, which is
    /// why the ones in this repository were converted when this was added.
    pub const BASE: Self = Self {
        id: "",
        name: "",
        description: "",
        kind: ParamKind::Text,
        required: false,
        multiple: false,
        choices: None,
    };
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

/// One keystroke, exactly as the device will send it.
///
/// # Why this crate spells the HID vocabulary again
///
/// It has no choice. Nothing here may depend on another Nobble crate — the rule
/// at the top of this module, and FR-044 behind it — so `nobble_core::HidAction`
/// is unreachable from an addon author's build, and Constitution VI's preferred
/// answer, generated bindings, does not cross a repository boundary that exists
/// on purpose (FR-043, FR-049). The alternative to restating the shape is not
/// restating it somewhere better; it is being unable to declare a keystroke at
/// all, and then FR-024 has no answer.
///
/// What *is* a choice is how much gets restated and how the copy is held in
/// step. These variants, their field names and their serde spelling in
/// [`KeystrokeDecl`](crate::protocol::KeystrokeDecl) match
/// `nobble_rpc::ActionDto::HidTap` and `HidConsumer` one for one, so the object
/// crossing this pipe is the same JSON text a binding is saved as, and the
/// daemon's conversion is a rename-free `match` a reader can check by eye. A
/// test in `nobble-service` — the only crate that can see both — pins that, and
/// stands in for the generated binding.
///
/// **That is weaker than one definition and is recorded as such rather than
/// argued away.** Nothing makes a new `HidAction` variant a compile error here,
/// because Cargo cannot see across the boundary. Two things bound the damage:
/// the drift is asymmetric — a variant added there narrows what this can say, a
/// variant added here breaks the daemon's exhaustive bridge — and what is copied
/// is USB HID's vocabulary rather than Nobble's, so it is not a format anyone
/// here is free to change.
///
/// # Why exactly these two, and why not MIDI
///
/// The cut is made by an existing function rather than by judgement.
/// `ActionDto::from_binding` has arms for `HidTap` and `HidConsumer`, and its
/// `_ => return None` covers key sequences and mouse movement, which have no
/// on-disk form — so an addon declaring one would declare a binding
/// `check_supported` refuses to save, and it fails the **whole file** rather
/// than the one key.
///
/// The narrower reading matters as much. One chord cannot type a string, and a
/// sequence is precisely the mechanism the contract rules out when it says an
/// addon able to ask the device to send anything at any time *"would be a
/// keylogger with extra steps"*. Excluding it in the type leaves no check for
/// anyone to forget.
///
/// MIDI is left out on different grounds, and the omission is **not** a claim
/// that device-resolved means keyboard. `Binding::Midi` is device-resolved too
/// and a control change is inherently continuous, so resolution and trigger are
/// genuinely independent axes. But `midi_note` and `midi_cc` are already
/// first-class binding kinds with their own editors, so there is no
/// discoverability gap for an addon to close. Adding a variant later is
/// additive; adding one now is speculative.
///
/// Deliberately **not** `#[non_exhaustive]` — the opposite choice from
/// [`ParamKind`], for the reason that separates them: an unfamiliar parameter
/// kind has a useful fallback, a text box, and an unfamiliar keystroke has none.
/// A wildcard arm here is how a device silently sends nothing. Adding a variant
/// *should* fail to compile in the daemon that has to translate it, which is
/// what [`Request`](crate::protocol::Request) says for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceKeystroke {
    /// Press and release one key, with modifiers held.
    ///
    /// **What is declared here is a suggestion the user is expected to change**,
    /// and an addon that treats it as a contract has misunderstood the field.
    /// Discord ships no keybind for Toggle Mute — the user invents one — so
    /// whatever is declared has to be transcribed into Discord by hand anyway.
    /// [`Self::Consumer`] is the opposite case: there the addon knows the answer.
    Tap {
        /// A HID usage code, **not** a character — the same distinction
        /// `HidKey` makes on the daemon side, for the same reason: a keystroke
        /// authored on a QWERTZ layout and pressed on a QWERTY host produces a
        /// different character, and hiding that makes it invisible until
        /// somebody complains. Usage `0x10` is the key labelled **M** on ANSI
        /// and the key labelled **,** on AZERTY, so an addon's choice of chord
        /// can collide with something on a layout its author never saw. That is
        /// what the editor is for.
        key: u8,
        /// Held with it.
        ctrl: bool,
        /// Held with it.
        shift: bool,
        /// Held with it.
        alt: bool,
        /// Held with it — Windows, Command, Super.
        gui: bool,
    },
    /// A Consumer Control usage — the media keys.
    ///
    /// Here because [ADR-0021] intends `media` to gain a device-resolved
    /// play/pause as a *new* action, and calls its case stronger than Discord's:
    /// a consumer usage is layout-free and needs nothing arranged elsewhere, so
    /// the addon genuinely knows the number and the declaration is a fact rather
    /// than a suggestion. A type that could not say so would have made the
    /// adoption impossible.
    ///
    /// [ADR-0021]: ../../../docs/decisions/0021-addon-device-resolved-actions.md
    Consumer {
        /// The usage.
        usage: u16,
    },
}

/// One thing an addon *names* and the **device** does, with no addon running.
///
/// [ADR-0021]. Discord's Toggle Mute is the case this exists for: it is a
/// *global* keybind, so a device that sends it controls Discord from the
/// background — at the login screen, inside a full-screen game, and with Nobble
/// closed. That is Principle V and [ADR-0007], and a stronger promise than any
/// addon can otherwise make.
///
/// Binding one writes a HID binding. [`Addon::perform`] is never called, the
/// addon need not be running, and it need not ever have been *allowed* to run —
/// ADR-0021 exempts these from the permission grant, because the grant's only
/// lever is *do not start the process* and a keystroke in flash starts none.
/// That is defensible only because what the device will send is disclosed when
/// the key is bound and frozen there.
///
/// # Why this is not [`AddonAction`] with more fields
///
/// Because the two are not the same shape, and one struct would carry fields
/// that are load-bearing in one regime and meaningless in the other, with
/// nothing but a doc comment saying which.
///
/// A device-resolved action has no [`Trigger`]: a keystroke has no position to
/// send, so continuous is incoherent *by construction* rather than merely
/// disallowed. It has no [`AddonParam`]s: a resolved binding carries no
/// parameters at all, so a runtime-varying one is impossible here rather than
/// late. ADR-0021 calls both impossible, and this is the shape in which they
/// cannot be *written down* — which is the discipline `Binding` already states
/// for itself, that the kind and the payload cannot be constructed disagreeing.
///
/// The honest limit of that: an addon written in something other than Rust can
/// still put `"trigger"` and `"params"` in the JSON, because serde ignores
/// fields it was not asked about. What it cannot do is make them *mean*
/// anything — there is nowhere for the daemon to read them from, so there is no
/// check to forget. That is a weaker claim than "unrepresentable" and it is the
/// true one.
///
/// The cost of a second list is real and is paid in one place: **ids share one
/// namespace with [`Addon::actions`]**, because a binding stores one string and
/// cannot say which list it came from. A collision has no type to prevent it,
/// and the daemon must refuse it by name rather than guess — otherwise binding
/// the device-resolved twin resolves to the host-resolved one, which is the
/// *"silently fall back to a host-resolved call when the daemon happens to be
/// running"* the contract forbids outright. Worth knowing that ids within
/// [`Addon::actions`] are not checked for uniqueness today either, and
/// `Registry::perform` takes the first match; this makes an existing unenforced
/// invariant load-bearing rather than inventing a new hazard.
///
/// # A separate list here is not a separate group in the interface
///
/// ADR-0021 requires these to appear in the addon's action list *alongside* the
/// host-resolved ones, reported per row rather than segregated, because grouping
/// by mechanism is what FR-024b forbids in the very addon that will hold both
/// families. Nothing here decides that: the daemon assembles one list for the
/// interface out of both, the way it already computes `applies` per row rather
/// than reading it from a declaration. What an author writes and what a user
/// reads have never been the same shape.
///
/// [ADR-0021]: ../../../docs/decisions/0021-addon-device-resolved-actions.md
/// [ADR-0007]: ../../../docs/decisions/0007-input-delivery.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceAction {
    /// Stable identifier, and what the binding records as provenance so that
    /// removing the addon reports the binding broken rather than leaving a bare
    /// keystroke nobody can explain (`003-FR-066` with `003-FR-047`). It must
    /// not change once anyone could have bound it, and it must not collide with
    /// an [`AddonAction::id`] on the same addon.
    pub id: &'static str,
    /// What to call it in a menu. The whole reason not to leave this as a
    /// hand-configured HID binding: the daemon's own summary of a raw tap is
    /// `Ctrl+Shift+usage 0x10`, which is accurate and unexplainable.
    pub name: &'static str,
    /// What it does, in a sentence.
    pub description: &'static str,
    /// What the device sends **by default**. The user can change it, and must be
    /// able to, because it has to match whatever they set in the other
    /// application and only they know what that is.
    ///
    /// Read once, when the key is bound, and never re-resolved on load
    /// (ADR-0021). So a later version of the addon cannot change what an
    /// already-bound key types — and a wrong default is wrong for everyone who
    /// already bound it, with no upgrade path short of rebinding. That is the
    /// right way round: the alternative is an addon update silently altering a
    /// key the user authored.
    pub keystroke: DeviceKeystroke,
    /// The step outside Nobble that has to happen for this to work, if there is
    /// one: *"Set this keybind in Discord: User Settings > Keybinds."*
    ///
    /// Free text, shown at the moment of binding, and **never parsed** — which
    /// is what lets it also carry a suggested chord in prose without that
    /// suggestion becoming a grammar with a compatibility surface of its own.
    /// The day something reads a keystroke out of this string, this field has
    /// quietly become the interface [`DeviceKeystroke`] exists to be instead.
    ///
    /// `None` where there is nothing to arrange elsewhere, which is the case a
    /// future device-resolved play/pause is in. An [`Option`] rather than an
    /// empty string, because the two states oblige the interface differently:
    /// `Some` means it must not claim the key works until the user confirms the
    /// step (FR-024), since Nobble cannot read another application's settings
    /// and must say so rather than pretend. A sentinel would make *the author
    /// left it blank* and *there is nothing to do* the same value.
    pub prerequisite: Option<&'static str>,
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
    params: &'a BTreeMap<String, ParamValue>,
    value: Option<u16>,
    input: Option<&'a str>,
}

/// Nothing configured, for an action that takes no parameters.
static NO_PARAMS: std::sync::LazyLock<BTreeMap<String, ParamValue>> =
    std::sync::LazyLock::new(BTreeMap::new);

impl<'a> Invocation<'a> {
    /// A press, with the binding's parameters.
    #[must_use]
    pub fn press(params: &'a BTreeMap<String, ParamValue>) -> Self {
        Self {
            params,
            value: None,
            input: None,
        }
    }

    /// A position, with the binding's parameters.
    #[must_use]
    pub fn moved(params: &'a BTreeMap<String, ParamValue>, value: u16) -> Self {
        Self {
            params,
            value: Some(value),
            input: None,
        }
    }

    /// A press with nothing configured. For tests, and for actions that take
    /// no parameters.
    #[must_use]
    pub fn bare() -> Self {
        Self {
            params: &NO_PARAMS,
            value: None,
            input: None,
        }
    }

    /// Say which input this came from.
    ///
    /// A builder rather than a fourth argument, so every existing call still
    /// compiles and reads the same. The host adds it; an addon never does.
    #[must_use]
    pub fn from(mut self, input: &'a str) -> Self {
        self.input = Some(input);
        self
    }

    /// Which input fired, as the same opaque string
    /// [`Addon::bound_inputs`](crate::Addon::bound_inputs) lists.
    ///
    /// `None` when the interface asked rather than a key: there is no input,
    /// and inventing one would name a key that does not exist.
    ///
    /// **Opaque, and meant to stay that way.** It is a key to match against the
    /// ordered list, not something to parse — the *order* is the daemon's
    /// answer, because only the daemon knows where the modules physically are.
    #[must_use]
    pub fn input(&self) -> Option<&str> {
        self.input
    }

    /// One parameter, if the binding set it.
    ///
    /// Absent and empty are the same answer here. A text box someone cleared
    /// stores `""`, and an addon checking only for absence would then treat a
    /// deliberately blank field as a configured one.
    /// **A parameter holding a list reads as absent here**, rather than as its
    /// first entry. See [`ParamValue::one`]: an addon that declared one value
    /// and silently used the first of several would mute one of the three
    /// applications somebody named and report nothing.
    #[must_use]
    pub fn param(&self, id: &str) -> Option<&str> {
        self.params.get(id).and_then(ParamValue::one)
    }

    /// Every value of one parameter, whether it holds one or several.
    ///
    /// The accessor a `multiple` parameter reads. A single value reads as a
    /// list of one, so an addon does not have to care how the user happened to
    /// fill it in — and a configuration written before lists existed contains
    /// exactly that case.
    #[must_use]
    pub fn param_all(&self, id: &str) -> Vec<&str> {
        self.params.get(id).map(ParamValue::all).unwrap_or_default()
    }

    /// The map as it was stored, for a host that has to forward it verbatim.
    ///
    /// Not for addons: an addon wants [`Self::param`] or
    /// [`Self::param_all`]. This exists because the daemon builds an
    /// invocation on one side of a pipe and has to put the same thing back on
    /// the wire on the other, and re-deriving it from the accessors would turn
    /// a list into whatever the accessors happened to flatten it to.
    #[must_use]
    pub fn raw_params(&self) -> &BTreeMap<String, ParamValue> {
        self.params
    }

    /// Every parameter, for reporting.
    ///
    /// A list renders comma-separated, because this feeds a log line rather
    /// than anything that parses it back.
    pub fn params(&self) -> impl Iterator<Item = (&str, String)> {
        self.params
            .iter()
            .map(|(k, v)| (k.as_str(), v.all().join(", ")))
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

/// Something an addon needs to be allowed to do (FR-046).
///
/// Declared as a set the addon needs *in order to work*, not as a wish list:
/// the user grants or refuses the whole declaration, because a half-granted
/// addon is a matrix of broken states nobody asked for and every one of them
/// would have to be designed.
///
/// # Only one of these is enforced, and this type does not pretend otherwise
///
/// [`Self::Credentials`] is real: the daemon holds the credential store and an
/// addon can only ask, so a refusal is a refusal. The other three are
/// **declarations shown to the user**, and an ungranted addon is simply not
/// started — which is a genuine control, because a process that is not running
/// opens no sockets. What is *not* true is that a *running* addon is confined
/// to what it declared. Nothing stops a granted addon reaching a host it never
/// mentioned.
///
/// That gap is [ADR-0016]'s, deliberately, and it is named in the interface
/// rather than papered over: "may reach api.spotify.com" must not be read as
/// "and nothing else" until platform confinement makes it true.
///
/// [ADR-0016]: ../../../docs/decisions/0016-addon-process-boundary.md
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Permission {
    /// Reach a host over the network.
    ///
    /// One host per declaration rather than a list, so the interface can show
    /// them as separate lines and so a diff between two versions of an addon
    /// reads as "it now also wants X".
    Network {
        /// The host, as it appears in a URL: `api.spotify.com`.
        host: &'static str,
        /// Why, in a few words, shown next to it. An addon that cannot explain
        /// what it wants a host for is asking the user to guess.
        reason: &'static str,
    },
    /// Read or write files under a path.
    Files {
        /// The directory or file, as a path the user would recognise.
        path: &'static str,
        /// Whether it writes, or only reads. The distinction is the whole
        /// difference between "reads your project list" and "can delete it".
        write: bool,
        /// Why.
        reason: &'static str,
    },
    /// Start another program.
    Launch {
        /// What it starts. `the default browser` is a legitimate answer here —
        /// this is shown to a person, not matched against anything.
        program: &'static str,
        /// Why.
        reason: &'static str,
    },
    /// Keep credentials of its own, in the OS credential store.
    ///
    /// The enforced one. Ungranted, the daemon answers every credential ask
    /// with a refusal, which an addon must survive: it is the same answer as
    /// "nothing stored yet", and an addon that handles being signed out
    /// already handles this.
    Credentials {
        /// Why. "To stay signed in to your Spotify account" — the account is
        /// the thing the user is actually deciding about.
        reason: &'static str,
    },
}

impl Permission {
    /// Why the addon says it needs this.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::Network { reason, .. }
            | Self::Files { reason, .. }
            | Self::Launch { reason, .. }
            | Self::Credentials { reason } => reason,
        }
    }
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

    /// Everything it *names* that the device does on its own (ADR-0021).
    ///
    /// Defaulted to nothing, like [`Self::signals`] and [`Self::settings`], and
    /// for a stronger reason than either: almost every addon has none, and the
    /// whole point of the declaration is that it is exceptional. An addon
    /// listing one is saying *this is better as a keystroke than as a call to
    /// me, and here is what to call it*.
    ///
    /// **[`Self::perform`] is never called for one of these** by a daemon that
    /// understands the declaration — it resolved the action to a keystroke when
    /// the key was bound, and the press never reaches the host at all. An author
    /// implements nothing for them and must not try. A daemon built against an
    /// older SDK dropped this field on the way in and will call `perform`
    /// anyway; [`run`](crate::run) answers that itself with a failure naming the
    /// cause, so an author still implements nothing.
    ///
    /// Ids share one namespace with [`Self::actions`], because a binding stores
    /// one string and cannot say which list it came from.
    fn device_actions(&self) -> &'static [DeviceAction] {
        &[]
    }

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

    /// The inputs bound to one of this addon's actions, **in device order**.
    ///
    /// Sent when it changes: a binding edited, a profile switched, a module
    /// attached or unplugged. Defaulted to ignoring it, because almost no addon
    /// cares which key called it.
    ///
    /// # Why the daemon sends an order rather than positions
    ///
    /// `006-FR-014a` defines fader order as *"ascending slot position, then
    /// input index within the module"* — and the only side that knows where a
    /// module physically sits is the daemon, which owns the inventory. Sending
    /// coordinates would make every addon that cares reimplement the sort, and
    /// the second implementation would be the one that was wrong about a
    /// module attached at a negative offset.
    ///
    /// So this is already sorted. Match [`Invocation::input`] against it to
    /// learn which fader moved and where it sits among the others.
    fn bound_inputs(&mut self, action: &str, inputs: &[String]) {
        let _ = (action, inputs);
    }

    /// Named lists its parameters can draw options from (ADR-0022).
    ///
    /// Defaulted to nothing, because most parameters are free text and always
    /// were. Declared here rather than on the parameter so that two parameters
    /// can share one list — a pin and a priority pointing at the same people
    /// must not be able to show two different rosters.
    ///
    /// A source marked [`AddonChoices::live`] carries no values here; the
    /// daemon asks for them when somebody opens the picker.
    fn choices(&self) -> &'static [AddonChoices] {
        &[]
    }

    /// The current options for a [`live`](AddonChoices::live) source.
    ///
    /// Asked **when somebody opens the picker, and at no other time.** Not
    /// polled: a timer would cost idle CPU for a picker nobody has open. So
    /// this is allowed to be the slow one — but it still must not block for
    /// long, because an interface is waiting on it with nothing to draw.
    ///
    /// Answer from whatever the addon already knows rather than by asking the
    /// far side. An addon that holds a cached picture answers from memory; one
    /// that must fetch should keep the request small and give up quickly rather
    /// than leave a menu spinning.
    ///
    /// **The order is the addon's to decide**, and that is the point of a named
    /// source rather than a per-parameter one: an addon that wants *who is here
    /// now*, then *who I have seen before*, then nothing, returns one flat
    /// ordered list and the interface renders it in that order. No interface
    /// support is needed for a concept only the addon has.
    ///
    /// An unknown id answers empty rather than panicking — the daemon and the
    /// addon can disagree across a version, and a menu with nothing in it is a
    /// better outcome than a dead child process.
    fn live_choices(&mut self, _id: &str) -> Vec<Choice> {
        Vec::new()
    }

    /// Everything it needs to be *allowed* to do (FR-046).
    ///
    /// Defaulted to nothing, and that default is the honest one for more
    /// addons than it looks: an addon that drives a local API — the media
    /// session, the audio mixer — reaches no host, opens no file and keeps no
    /// account, so it has nothing to ask for and the user is never asked.
    ///
    /// **An addon declaring anything here does not run until the user grants
    /// it.** So the list is what the addon needs, not what it might one day
    /// like: every entry is a question somebody has to answer before the addon
    /// works at all, and an addon that asks for more than it uses is training
    /// people to grant without reading.
    fn permissions(&self) -> &'static [Permission] {
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

#[cfg(test)]
mod tests {
    use super::{ParamKind, Trigger, app_matches};

    /// Every variant survives the round trip its own encoder produces.
    ///
    /// The list is written out rather than iterated, because there is no
    /// `Trigger::all()` and inventing one to test with would be a second place
    /// that has to know every variant — the failure this whole arrangement is
    /// about. A new variant makes `as_wire` fail to compile; this is here so
    /// that a *renamed* one, which compiles perfectly, fails something.
    #[test]
    fn a_trigger_survives_its_own_spelling() {
        for t in [Trigger::Momentary, Trigger::Continuous] {
            assert_eq!(Trigger::from_wire(t.as_wire()), Some(t), "{t:?}");
        }
        assert_eq!(Trigger::from_wire("Continuous"), None, "case matters");
        assert_eq!(Trigger::from_wire(""), None);
    }

    /// The tokens themselves, pinned. The round trip above agrees with itself
    /// whatever both halves are renamed to; these two strings are compared
    /// against by a daemon decoding an addon's declaration and by TypeScript in
    /// the settings window, neither of which this crate can see.
    #[test]
    fn the_trigger_tokens_are_these_two() {
        assert_eq!(Trigger::Momentary.as_wire(), "momentary");
        assert_eq!(Trigger::Continuous.as_wire(), "continuous");
    }

    #[test]
    fn a_param_kind_survives_its_own_spelling() {
        for k in [ParamKind::Text, ParamKind::App] {
            assert_eq!(ParamKind::from_wire(k.as_wire()), Some(k), "{k:?}");
        }
        assert_eq!(ParamKind::from_wire("colour"), None);
    }

    #[test]
    fn the_param_kind_tokens_are_these_two() {
        assert_eq!(ParamKind::Text.as_wire(), "text");
        assert_eq!(ParamKind::App.as_wire(), "app");
    }

    #[test]
    fn an_application_matches_whichever_form_each_side_is_in() {
        // The two sides come from different places and neither will change:
        // Nobble's foreground watcher says `exe:spotify.exe`, and the API the
        // addon is asking says whatever the application registered with it.
        assert!(app_matches("Spotify.exe", "exe:spotify.exe"));
        assert!(app_matches(
            "SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify",
            "exe:spotify.exe"
        ));
        assert!(app_matches("Spotify.exe", "spotify"));
        assert!(app_matches("chrome.exe", "exe:Chrome.exe"));
    }

    #[test]
    fn a_different_application_does_not_match() {
        assert!(!app_matches("chrome.exe", "exe:spotify.exe"));
        assert!(!app_matches("Spotify.exe", "exe:firefox.exe"));
    }

    #[test]
    fn an_empty_target_matches_nothing_rather_than_everything() {
        // Naming a target means "this one". A blank one matching everything
        // would turn a mistyped field into a key that controls whatever
        // happens to be loudest, which is the failure the parameter exists to
        // prevent.
        assert!(!app_matches("Spotify.exe", ""));
        assert!(!app_matches("Spotify.exe", "exe:"));
    }
}
