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
pub const PROTOCOL: Version = Version { major: 1, minor: 0 };

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
        params: BTreeMap<String, String>,
        /// The fader position for a continuous action, absent for a press.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<u16>,
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
    Applies {
        /// Whether it is worth offering.
        applies: bool,
    },
    /// Answer to [`Request::Status`].
    Status {
        /// The sentence, if there is one.
        status: Option<String>,
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
    /// Everything it publishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<SignalDecl>,
    /// Everything it needs configuring.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<SettingDecl>,
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
            params: BTreeMap::from([("app".to_owned(), "exe:spotify.exe".to_owned())]),
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

    #[test]
    fn a_major_mismatch_is_incompatible_and_a_minor_one_is_not() {
        assert!(PROTOCOL.compatible_with(Version {
            major: 1,
            minor: 99
        }));
        assert!(!PROTOCOL.compatible_with(Version { major: 2, minor: 0 }));
    }
}
