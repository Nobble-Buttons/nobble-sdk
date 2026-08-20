//! What crosses the pipe between the daemon and an addon.
//!
//! [ADR-0016](../../../docs/decisions/0016-addon-process-boundary.md). One JSON
//! object per line, in both directions. Line-delimited rather than
//! length-prefixed or a binary codec because this is a **published** interface:
//! it has to be readable in a log, reproducible with `echo`, and implementable
//! by an addon author who has not installed a code generator.
//!
//! # Two shapes for one declaration, and why that is not two definitions
//!
//! The trait declares actions as `&'static [AddonAction]`, which is pleasant to
//! write as a `const` and impossible to deserialise. So the types here are
//! owned mirrors — [`ActionDecl`] beside [`AddonAction`](crate::AddonAction),
//! and so on.
//!
//! They are not two things to keep in step. The author writes the borrowed one
//! *once*; [`run`](crate::run) derives the owned one from it at the boundary.
//! Nothing hand-maintains the second, so the failure Constitution VI is about —
//! two definitions drifting — has nowhere to happen.
//!
//! # Both directions, and why the addon speaks first only about credentials
//!
//! Almost everything is the daemon asking and the addon answering. The one
//! exception is [`Ask`], which an addon sends *while* answering, when it needs
//! one of its own credentials. That makes the conversation strictly nested —
//! request, optional asks and their answers, reply — which is what lets
//! [`Credentials`](crate::Credentials) stay a plain synchronous call in the
//! addon's code rather than infecting the trait with futures.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The protocol version this SDK speaks.
///
/// Independent of the daemon's version, which is the whole point of FR-049: an
/// addon is built against an SDK, not against a daemon, and the two ship
/// separately. `major` differing means incompatible, and the daemon refuses
/// rather than loading — FR-049 again, with a message naming the fix, the same
/// rule the UI–daemon handshake already follows.
///
/// # 1.1 — device-resolved actions
///
/// [`Description::device_actions`] was added ([ADR-0021]). Minor, because
/// [`Version::compatible_with`] compares majors, the field is omitted when
/// empty, and an addon that declares none puts **not one new byte** on the wire.
/// A major bump would refuse every 1.1 addon on a 1.0 daemon — the shipped ones,
/// and every third-party addon that declares nothing of the kind — to defend
/// against one field.
///
/// **Additive on the wire is not additive in meaning**, and the version number
/// cannot carry the difference. Two things carry it instead, and neither is
/// optional:
///
/// - A daemon that reads this field must **refuse an action it cannot
///   represent, naming it**, rather than reading it as something else. The
///   refusal is per *action*: the addon stays in the registry with that one row
///   reported unusable. Failing the handshake instead would leave the addon
///   nowhere but a log line, which is a Principle IV collapse — and it is why
///   [`KeystrokeDecl`] carries [`KeystrokeDecl::Unknown`] rather than failing
///   the parse of the whole [`Description`].
/// - A daemon built against 1.0 does not know the field exists. Serde drops what
///   it does not recognise, so it will send [`Request::Perform`] for an action
///   whose author never wrote a `perform` arm. [`run`](crate::run) answers that
///   itself with a failure naming the cause, rather than letting it fall through
///   to `NoSuchAction`, which would send the user looking for a missing action
///   that is right there in the list.
///
/// # 1.2 — a prerequisite on an action
///
/// See [`AddonAction`]'s own history; recorded here so the list has no gap.
///
/// # 1.3 — an action the account cannot perform
///
/// [`Reply::Applies`] gained `unavailable` ([ADR-0030]). Minor for the same
/// reason 1.1 was: the field is `#[serde(default)]` and skipped when `None`, so
/// a 1.2 addon — which never sends it — parses unchanged and puts **not one new
/// byte** on the wire.
///
/// The asymmetry worth knowing: a **1.3 daemon with a 1.2 addon** reads `None`
/// and offers everything, which is exactly the behaviour before this field
/// existed. A **1.2 daemon with a 1.3 addon** drops the field it does not know,
/// and the user gets the refusal on press rather than the mark in advance —
/// degraded, never wrong. Neither direction needed a new request, which is the
/// point: the per-action round trip already existed and now answers two
/// questions.
///
/// [ADR-0021]: ../../../docs/decisions/0021-addon-device-resolved-actions.md
/// [ADR-0030]: ../../../docs/decisions/0030-actions-an-account-cannot-perform.md
pub const PROTOCOL: Version = Version { major: 1, minor: 3 };

/// A protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    /// Incompatible when this differs.
    pub major: u16,
    /// Additive; a lower minor on either side is fine.
    pub minor: u16,
}

impl Version {
    /// Whether these two can talk.
    #[must_use]
    pub fn compatible_with(self, other: Self) -> bool {
        self.major == other.major
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// What the daemon asks an addon to do.
///
/// Deliberately **not** `#[non_exhaustive]`, matching `DeviceEvent` and
/// `Effect` in the daemon and for the same reason: a wildcard arm is how a
/// newly added request gets silently ignored. Adding one should fail to compile
/// in every implementation that has to answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "ask", rename_all = "snake_case")]
pub enum Request {
    /// Version handshake. Always first.
    Hello {
        /// What the daemon speaks.
        version: Version,
    },
    /// Everything static about the addon: what it is and what it offers.
    ///
    /// One request rather than six, because none of it changes while the addon
    /// runs and six round trips at startup would be six chances to be half
    /// described.
    Describe,
    /// Whether it could act right now.
    Availability,
    /// Whether one action is worth offering at the moment.
    Applies {
        /// Which one.
        action: String,
    },
    /// A sentence for the interface, if it has one.
    Status,
    /// The current options for a live choice source (ADR-0022).
    ///
    /// Sent when somebody opens a picker, and at no other time — never on a
    /// timer, which would cost idle CPU for a menu nobody has open.
    LiveChoices {
        /// Which named source.
        id: String,
    },
    /// Do something.
    Perform {
        /// Which action.
        action: String,
        /// What the binding was configured with (ADR-0012).
        ///
        /// Omitted when empty, and `value` when absent. Most performs are a
        /// bare press, and `"params":{},"value":null` on every one of them is
        /// noise in a log somebody is reading to find out what happened.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        params: BTreeMap<String, crate::addon::ParamValue>,
        /// The fader position for a continuous action, absent for a press.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<u16>,
    },
    /// The inputs bound to one action, in device order (`006-FR-014a`).
    ///
    /// Sent unprompted whenever the set or its order changes, which is the one
    /// place the daemon tells an addon something rather than asking it. It is
    /// still a request on the wire — the addon answers `Done` — because a
    /// second message shape would need a second reader at both ends.
    BoundInputs {
        /// Which of the addon's actions.
        action: String,
        /// Every input bound to it, already sorted.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        inputs: Vec<String>,
    },
    /// Which of one action's inputs the user is holding right now.
    ///
    /// Sent when the set changes. Empty is the ordinary case and is omitted on
    /// the wire, so an addon that never receives one has nothing held — which
    /// is also what an older host that never sends one means.
    HeldInputs {
        /// Which of the addon's actions.
        action: String,
        /// The inputs currently held, in the same vocabulary as
        /// [`Self::BoundInputs`].
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        held: Vec<String>,
    },
    /// Take these settings.
    Configure {
        /// By setting id.
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        values: BTreeMap<String, String>,
    },
    /// Read every signal it publishes, now.
    ReadSignals,
    /// Stop. The addon should exit; the daemon kills it if it does not.
    Shutdown,
}

/// What an addon says back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "say", rename_all = "snake_case")]
pub enum Reply {
    /// Answer to [`Request::Hello`].
    Welcome {
        /// What the addon's SDK speaks.
        version: Version,
    },
    /// Answer to [`Request::Describe`].
    Description(Description),
    /// Answer to [`Request::Availability`].
    Availability(AvailabilityDecl),
    /// Answer to [`Request::Applies`].
    ///
    /// **Two facts, one round trip.** They are different questions — relevance
    /// and capability, see [`Addon::applies`] and [`Addon::unavailable`] — but
    /// they are both per-action and both re-read every time the interface
    /// draws. A second request would double the per-action cost of a refresh
    /// that already asks each child once per action, so the question stays
    /// `Applies` and the answer carries both.
    Applies {
        /// Whether it is worth offering.
        applies: bool,
        /// Why this account cannot perform it, if it cannot.
        ///
        /// Added in protocol 1.3. `default` rather than required, so a 1.2
        /// addon — which never sends it — still parses here, which is what
        /// makes this a minor bump.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        unavailable: Option<String>,
    },
    /// Answer to [`Request::Status`].
    Status {
        /// The sentence, if there is one.
        status: Option<String>,
    },
    /// Answer to [`Request::LiveChoices`].
    LiveChoices {
        /// In the order the addon wants them shown — the ordering is a concept
        /// only the addon has, so the interface renders rather than sorts.
        choices: Vec<ChoiceDecl>,
    },
    /// Answer to [`Request::ReadSignals`].
    Signals(ReadingDecl),
    /// It worked, and there is nothing to say. Answers `Perform`, `Configure`
    /// and `Shutdown`.
    Done,
    /// It did not work.
    Failed {
        /// Which kind, so the daemon can tell "no such action" from "the
        /// target is not running" without parsing prose.
        kind: FailureKind,
        /// What to tell the user. Already a sentence.
        detail: String,
    },
    /// The addon wants one of its own credentials, mid-request.
    ///
    /// Not really a reply: the daemon answers with [`Answer`] and the addon
    /// then carries on with the request it was already handling. It shares this
    /// channel because there is only one pipe.
    Ask(Ask),
}

/// Why an action failed, in the categories the daemon distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    /// The addon does not have that action. Usually a profile referring to
    /// something a newer or older build had — US9 §4 says preserve and report,
    /// never silently drop.
    NoSuchAction,
    /// It exists but cannot run now: the target application is closed, the
    /// account is signed out.
    Unavailable,
    /// It tried and it failed.
    Failed,
}

/// A credential request, from the addon to the daemon.
///
/// **There is no addon field, and that is the design.** See
/// [`Credentials`](crate::Credentials) for the hole this shape closes: the
/// daemon knows which addon is asking because it knows which child it is
/// talking to, so an addon cannot name someone else's.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "want", rename_all = "snake_case")]
pub enum Ask {
    /// Read one.
    Get {
        /// Which key.
        key: String,
    },
    /// Store one.
    Set {
        /// Which key.
        key: String,
        /// What to store.
        value: String,
    },
    /// Forget one.
    Clear {
        /// Which key.
        key: String,
    },
    /// Read one from the addon's own store (ADR-0027).
    ///
    /// A different place from the three above, not a different key space. Those
    /// reach the OS credential store, which is for values that are the whole of
    /// an account's authority; this reaches a store the daemon encrypts at rest
    /// and is for everything an addon wants to remember that is *not* a secret.
    StoreGet {
        /// Which key.
        key: String,
    },
    /// Write one to the addon's own store.
    ///
    /// **What crosses this pipe is plaintext.** The encryption is the daemon's
    /// and is not optional — an addon that had to remember to ask for it would
    /// eventually produce a file indistinguishable from one that had reasoned
    /// about it.
    StoreSet {
        /// Which key.
        key: String,
        /// What to store.
        value: String,
    },
    /// Forget one from the addon's own store.
    StoreClear {
        /// Which key.
        key: String,
    },
    /// Every key in the addon's own store.
    StoreKeys,
}

/// The daemon's answer to an [`Ask`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "answer", rename_all = "snake_case")]
pub enum Answer {
    /// What was stored, or nothing.
    Value {
        /// Absent means there is none, which is an ordinary state.
        value: Option<String>,
    },
    /// The write or the clear succeeded.
    Stored,
    /// Every key there is, for [`Ask::StoreKeys`].
    ///
    /// Empty is the answer for a store that has never been written, a store
    /// whose key material has been rotated away, and a platform with no
    /// implementation. All three mean *there is nothing here*, and an addon
    /// that treated them differently would be acting on a distinction it cannot
    /// verify (ADR-0027).
    Keys {
        /// In whatever order the store keeps them.
        keys: Vec<String>,
    },
    /// It did not.
    Refused {
        /// Why, as a sentence.
        detail: String,
    },
}

/// Everything static about an addon.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Description {
    /// Stable identifier, stored in bindings.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// What it integrates, in a sentence.
    pub description: String,
    /// Everything it can be asked to do.
    pub actions: Vec<ActionDecl>,
    /// Everything it *names* that the device does on its own (ADR-0021).
    ///
    /// Omitted when empty, which is almost always, and the omission is
    /// load-bearing in the same way `permissions`' is: an addon built against
    /// SDK 1.0 sends no field, and *declares none* is exactly the right reading
    /// of that. The reverse — a 1.1 addon talking to a 1.0 daemon — is the case
    /// the version number cannot express, and is answered in
    /// [`run`](crate::run) rather than here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub device_actions: Vec<DeviceActionDecl>,
    /// Everything it publishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<SignalDecl>,
    /// Everything it needs configuring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingDecl>,
    /// Named lists its parameters draw options from (ADR-0022).
    ///
    /// Omitted when empty, which is every addon that has none — and an addon
    /// built against an older SDK sends no field, which reads correctly as
    /// "declares none".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<ChoicesDecl>,
    /// Everything it needs to be allowed to do (FR-046).
    ///
    /// Omitted when empty, which is the ordinary case — and the omission is
    /// load-bearing rather than tidy: an addon built against an older SDK sends
    /// no field, and "declares nothing" is exactly the right reading of that.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<PermissionDecl>,
}

/// One permission, owned. Mirrors [`Permission`](crate::Permission).
///
/// Tagged by kind with the target as a separate field, rather than one string
/// per variant, so a consumer that does not recognise a future kind can still
/// show the user *something* — the reason, at least — instead of dropping a
/// permission silently. Dropping one is the failure that matters here: it
/// would understate what the addon asked for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PermissionDecl {
    /// Reach a host over the network.
    Network {
        /// The host.
        host: String,
        /// Why.
        reason: String,
    },
    /// Read or write files under a path.
    Files {
        /// The path.
        path: String,
        /// Whether it writes as well as reads.
        write: bool,
        /// Why.
        reason: String,
    },
    /// Start another program.
    Launch {
        /// What it starts.
        program: String,
        /// Why.
        reason: String,
    },
    /// Keep credentials in the OS credential store. The enforced one.
    Credentials {
        /// Why.
        reason: String,
    },
}

/// One action, owned. Mirrors [`AddonAction`](crate::AddonAction).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDecl {
    /// Stable identifier, stored in bindings.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// What it does, in a sentence.
    pub description: String,
    /// What kind of input it expects: `momentary` or `continuous`.
    pub trigger: String,
    /// What it can be configured with.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParamDecl>,
    /// What a person must do elsewhere first, if anything. Mirrors
    /// [`AddonAction::prerequisite`](crate::AddonAction::prerequisite).
    ///
    /// Present here because it was once not, and nobody noticed: the field
    /// existed on [`AddonAction`](crate::AddonAction), the daemon's RPC carried
    /// it, and the settings window rendered it — but this type sat in the middle
    /// and had no place to put it, so every out-of-process addon's prerequisite
    /// arrived as `None`. Unit tests on both sides passed throughout, because
    /// neither side crosses the pipe. Additive and optional, so an addon built
    /// against an older SDK still decodes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisite: Option<String>,
}

/// One device-resolved action, owned. Mirrors
/// [`DeviceAction`](crate::DeviceAction).
///
/// Deliberately **not** an [`ActionDecl`] with extra fields. It carries no
/// `trigger`, because a keystroke has no position to send, and no `params`,
/// because a resolved binding stores none — so the two combinations ADR-0021
/// calls impossible have nowhere to live rather than being checked for. A reader
/// of a log can also tell which kind they are looking at without checking a flag.
///
/// Serde ignores fields it was not asked about, so a hand-written JSON addon can
/// still *write* `"trigger"` beside one of these. What it cannot do is make it
/// mean anything, because nothing reads it. That is the accurate claim;
/// "unrepresentable" would be too strong.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceActionDecl {
    /// Stable identifier, stored as the binding's provenance. Shares one
    /// namespace with [`ActionDecl::id`]; the daemon refuses a collision by name
    /// rather than resolving it to whichever list it looked in first.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// What it does, in a sentence.
    pub description: String,
    /// The default the binding editor starts from, which the user may change.
    pub keystroke: KeystrokeDecl,
    /// The step outside Nobble the user must also take (FR-024), if any. Free
    /// text, shown at the moment of binding, and never parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prerequisite: Option<String>,
}

/// One keystroke, owned. Mirrors [`DeviceKeystroke`](crate::DeviceKeystroke).
///
/// # The tags are `nobble_rpc::ActionDto`'s tags, and that is the point
///
/// `hid_tap` and `hid_consumer`, with the same field names and the same
/// omit-when-false on the modifiers, so this object and the one a binding is
/// saved as are the **same JSON text**. The SDK cannot depend on `nobble-rpc`
/// (FR-044) and generated bindings do not cross that boundary, so identical
/// spelling plus a test pinning it is what stands in for Constitution VI's
/// single definition. If the two ever drift, that test is what says so — nothing
/// else will, because Cargo cannot see across the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum KeystrokeDecl {
    /// A keystroke.
    HidTap {
        /// HID usage code, not a character.
        key: u8,
        /// Held with it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        ctrl: bool,
        /// Held with it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        shift: bool,
        /// Held with it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        alt: bool,
        /// Held with it.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        gui: bool,
    },
    /// A Consumer Control usage — media keys.
    HidConsumer {
        /// The usage.
        usage: u16,
    },
    /// A kind this build has no name for.
    ///
    /// **Not a fallback, and never sent by this SDK** — a refusal cut down to
    /// the size ADR-0021 requires. Without it, an addon built against a later
    /// SDK fails to deserialise its whole [`Description`], the handshake fails,
    /// and the addon appears nowhere but a log line. With it, the cost is one
    /// action the daemon reports as declared-but-unusable. The daemon must never
    /// treat this as a keystroke; there is nothing here to send.
    ///
    /// **It covers an unrecognised *tag*, and only that.** A malformed payload —
    /// `{"type":"hid_tap","key":999}`, a missing `key`, a `keystroke` that is a
    /// string — still fails the parse of the whole [`Description`], because
    /// serde has no way to localise the error to one element. That is left as it
    /// is rather than papered over: this SDK cannot emit one, so producing one
    /// means an addon written in something else is wrong about the format, and a
    /// loud refusal is the right answer to that. The guarantee is about *future
    /// versions*, which is what ADR-0021 asked for; it is not a general
    /// tolerance of bad input, and claiming otherwise would be the kind of
    /// promise somebody later relies on.
    ///
    /// The opposite decision from [`DeviceKeystroke`](crate::DeviceKeystroke),
    /// which has no such variant on purpose: a *declaration* should fail to
    /// compile when the vocabulary grows, and a *decode* should not fail at all.
    #[serde(other)]
    Unknown,
}

/// One parameter, owned. Mirrors [`AddonParam`](crate::AddonParam).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParamDecl {
    /// Stable identifier.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// What it is for.
    pub description: String,
    /// What it holds, so an interface can offer the right editor.
    pub kind: String,
    /// Whether the action fails without it.
    pub required: bool,
    /// Whether it holds several values rather than one (ADR-0022).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiple: bool,
    /// The id of a [`ChoicesDecl`] this draws its options from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<String>,
}

/// One named list of options, owned. Mirrors [`AddonChoices`](crate::AddonChoices).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoicesDecl {
    /// Stable id within the addon.
    pub id: String,
    /// What the list is.
    pub name: String,
    /// Whether the values must be asked for rather than read from below.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub live: bool,
    /// Every value, for a declared source. Empty when live.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<ChoiceDecl>,
}

/// One option, owned. Mirrors [`AddonChoice`](crate::AddonChoice).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChoiceDecl {
    /// What gets stored.
    pub value: String,
    /// What the user reads.
    pub label: String,
    /// A second line, where the label alone is ambiguous.
    ///
    /// **Defaulted, so this stayed additive.** An addon built against an
    /// earlier SDK sends no such field and an older daemon ignores it, which is
    /// what an API that is a promise to strangers has to manage.
    ///
    /// Exists because display names are not unique: two people called Alex in
    /// one call is ordinary, and the identity underneath is a number nobody
    /// recognises. A binding attached to the wrong Alex works perfectly and
    /// passes every test.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub detail: String,
}

/// One signal, owned. Mirrors [`AddonSignal`](crate::AddonSignal).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDecl {
    /// Stable identifier, qualified by the addon id in a condition.
    pub id: String,
    /// What to call it.
    pub name: String,
    /// What it means.
    pub description: String,
}

/// One setting, owned. Mirrors [`AddonSetting`](crate::AddonSetting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingDecl {
    /// The parameter it is.
    pub param: ParamDecl,
    /// Whether it goes to the credential store rather than the settings file
    /// (ADR-0013). A secret's value never crosses this pipe on the way *out*.
    pub secret: bool,
}

/// Whether an addon can act, owned. Mirrors [`Availability`](crate::Availability).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum AvailabilityDecl {
    /// It could act right now.
    Ready,
    /// It could not, and this is why.
    Unavailable {
        /// A sentence for the user.
        detail: String,
    },
}

/// A signal reading, owned. Mirrors [`Reading`](crate::Reading).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "read", rename_all = "snake_case")]
pub enum ReadingDecl {
    /// What each signal says.
    Values {
        /// By signal id.
        values: Vec<(String, bool)>,
    },
    /// It could not answer, which FR-064 requires to be distinguishable from
    /// every signal reading false.
    Unavailable {
        /// A sentence for the user.
        detail: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The wire is JSON, so the shape is part of the published interface and a
    /// rename is a breaking change. Pinned as text rather than round-tripped,
    /// because a round trip agrees with itself no matter what it renamed.
    #[test]
    fn a_perform_looks_like_this_on_the_wire() {
        let json = serde_json::to_string(&Request::Perform {
            action: "play_pause".to_owned(),
            params: BTreeMap::from([("app".to_owned(), "exe:spotify.exe".into())]),
            value: None,
        })
        .expect("serialise");
        assert_eq!(
            json,
            r#"{"ask":"perform","action":"play_pause","params":{"app":"exe:spotify.exe"}}"#
        );
    }

    #[test]
    fn an_ask_carries_no_addon_name() {
        // The whole of the fix. If this ever gains an `addon` field, an addon
        // can name someone else's credentials again.
        let json = serde_json::to_string(&Ask::Get {
            key: "refresh_token".to_owned(),
        })
        .expect("serialise");
        assert_eq!(json, r#"{"want":"get","key":"refresh_token"}"#);
        assert!(!json.contains("addon"));
    }

    #[test]
    fn every_message_round_trips() {
        let requests = vec![
            Request::Hello { version: PROTOCOL },
            Request::Describe,
            Request::Availability,
            Request::Applies {
                action: "sign_out".to_owned(),
            },
            Request::Status,
            Request::Configure {
                values: BTreeMap::new(),
            },
            Request::ReadSignals,
            Request::Shutdown,
        ];
        for r in requests {
            let text = serde_json::to_string(&r).expect("serialise");
            let back: Request = serde_json::from_str(&text).expect("parse");
            assert_eq!(back, r, "{text}");
        }
    }

    /// The keystroke object has to be byte-identical to what
    /// `nobble_rpc::ActionDto::HidTap` writes into the configuration file. This
    /// pins one half; `nobble-service`, the only crate that can see both, pins
    /// the equality. Two crates forbidden to see each other agree here or
    /// nowhere.
    #[test]
    fn a_device_action_looks_like_this_on_the_wire() {
        let json = serde_json::to_string(&DeviceActionDecl {
            id: "discord_mute".to_owned(),
            name: "Toggle mute in Discord".to_owned(),
            description: "Mutes and unmutes your microphone in Discord.".to_owned(),
            keystroke: KeystrokeDecl::HidTap {
                key: 0x10,
                ctrl: true,
                shift: true,
                alt: false,
                gui: false,
            },
            prerequisite: Some("Set this keybind in Discord: User Settings > Keybinds.".to_owned()),
        })
        .expect("serialise");
        assert_eq!(
            json,
            r#"{"id":"discord_mute","name":"Toggle mute in Discord","description":"Mutes and unmutes your microphone in Discord.","keystroke":{"type":"hid_tap","key":16,"ctrl":true,"shift":true},"prerequisite":"Set this keybind in Discord: User Settings > Keybinds."}"#
        );
    }

    /// The claim that makes 1.1 a minor bump rather than a promise. An addon
    /// that declares nothing new must put nothing new on the wire, or every
    /// existing addon becomes a new conversation with a daemon that has not
    /// changed.
    #[test]
    fn an_addon_declaring_none_puts_not_one_new_byte_on_the_wire() {
        let json = serde_json::to_string(&Description {
            id: "probe".to_owned(),
            name: "Probe".to_owned(),
            description: "For tests.".to_owned(),
            actions: vec![],
            device_actions: vec![],
            choices: vec![],
            signals: vec![],
            settings: vec![],
            permissions: vec![],
        })
        .expect("serialise");
        assert_eq!(
            json,
            r#"{"id":"probe","name":"Probe","description":"For tests.","actions":[]}"#
        );
    }

    /// ADR-0021's refusal, sized. An addon built against a later SDK costs the
    /// daemon one unusable action, never the parse of the whole description —
    /// which would fail the handshake and leave the addon nowhere but a log
    /// line.
    ///
    /// Read inside a `Reply`, which is how it actually arrives: `Reply` is
    /// internally tagged, and an internally tagged outer enum buffers its
    /// content, which is exactly the situation where a `#[serde(other)]` inside
    /// might have behaved differently from the bare struct. It does not.
    #[test]
    fn a_keystroke_kind_from_the_future_costs_one_action_not_the_description() {
        let wire = r#"{"say":"description","id":"a","name":"A","description":"d","actions":[],"device_actions":[{"id":"x","name":"X","description":"d","keystroke":{"type":"hid_sequence","keys":[4,5]}}]}"#;
        let Reply::Description(d) = serde_json::from_str(wire).expect("parse") else {
            panic!("expected a description");
        };
        assert_eq!(d.device_actions.len(), 1);
        assert_eq!(d.device_actions[0].keystroke, KeystrokeDecl::Unknown);
        assert_eq!(d.device_actions[0].prerequisite, None);
    }

    /// The other half of the same guarantee, and the reason it is written down:
    /// [`KeystrokeDecl::Unknown`] catches an unrecognised **tag** and nothing
    /// else. A malformed payload fails the whole description, because serde
    /// cannot localise the error to one element.
    ///
    /// Asserted rather than left as a surprise. This SDK cannot emit any of
    /// these, so producing one means an addon written in something else is wrong
    /// about the format, and a loud refusal is the right answer — but somebody
    /// reading `Unknown`'s existence could reasonably assume more tolerance than
    /// there is, and act on it.
    #[test]
    fn a_malformed_keystroke_still_fails_the_whole_description() {
        for (case, wire) in [
            (
                "no keystroke at all",
                r#"{"id":"a","name":"A","description":"d","actions":[],"device_actions":[{"id":"x","name":"X","description":"d"}]}"#,
            ),
            (
                "a string where the object goes",
                r#"{"id":"a","name":"A","description":"d","actions":[],"device_actions":[{"id":"x","name":"X","description":"d","keystroke":"ctrl+shift+m"}]}"#,
            ),
            (
                "a tag with no usage",
                r#"{"id":"a","name":"A","description":"d","actions":[],"device_actions":[{"id":"x","name":"X","description":"d","keystroke":{"type":"hid_tap"}}]}"#,
            ),
            (
                "a usage that is not a byte",
                r#"{"id":"a","name":"A","description":"d","actions":[],"device_actions":[{"id":"x","name":"X","description":"d","keystroke":{"type":"hid_tap","key":999}}]}"#,
            ),
        ] {
            assert!(
                serde_json::from_str::<Description>(wire).is_err(),
                "{case} parsed, and it must not"
            );
        }
    }

    #[test]
    fn a_major_mismatch_is_incompatible_and_a_minor_one_is_not() {
        assert!(PROTOCOL.compatible_with(Version {
            major: 1,
            minor: 99
        }));
        assert!(!PROTOCOL.compatible_with(Version { major: 2, minor: 0 }));
    }
}
