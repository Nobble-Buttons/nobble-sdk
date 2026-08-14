//! Write a Nobble addon.
//!
//! An addon is whatever knows how to make another application do something —
//! skip a track, switch an OBS scene, start a Fusion 360 command. You implement
//! [`Addon`], hand it to [`run`], and the daemon does the rest.
//!
//! ```no_run
//! use nobble_addon_sdk::{Addon, AddonAction, AddonError, Availability, Invocation, Trigger, run};
//!
//! struct Hello;
//!
//! const ACTIONS: &[AddonAction] = &[AddonAction {
//!     id: "wave",
//!     name: "Wave",
//!     description: "Says hello.",
//!     trigger: Trigger::Momentary,
//!     params: &[],
//! }];
//!
//! impl Addon for Hello {
//!     fn id(&self) -> &'static str { "hello" }
//!     fn name(&self) -> &'static str { "Hello" }
//!     fn description(&self) -> &'static str { "An addon that waves." }
//!     fn actions(&self) -> &'static [AddonAction] { ACTIONS }
//!     fn availability(&self) -> Availability { Availability::Ready }
//!     fn perform(&mut self, action: &str, _i: &Invocation<'_>) -> Result<(), AddonError> {
//!         match action {
//!             "wave" => Ok(()),
//!             other => Err(AddonError::NoSuchAction(other.to_owned())),
//!         }
//!     }
//! }
//!
//! fn main() { run(Hello); }
//! ```
//!
//! # Why this is a separate process
//!
//! [ADR-0016](../../../docs/decisions/0016-addon-process-boundary.md). An addon
//! runs as a child of the daemon and speaks the protocol in [`protocol`] over
//! its stdin and stdout. That is what makes FR-045 true — a crash, a hang or a
//! leak in your addon cannot reach input, MIDI, or anybody else's addon — and
//! it is what lets [`Credentials`] be scoped to you rather than handed out as a
//! key to everyone's.
//!
//! You do not write any of that. [`run`] owns the loop and the framing; you
//! write the trait.
//!
//! # What is deliberately not here
//!
//! No device, input, layout or module types. An addon contributes actions,
//! signals, settings and configuration UI (FR-047) and has no business knowing
//! what an `InputId` is. If you find yourself needing one, that is a gap in
//! this SDK to be reported rather than routed around — FR-050 says official
//! addons get no private interfaces either, so the gap is real for everyone.

mod addon;
pub mod protocol;
mod run;

pub use addon::{
    Addon, AddonAction, AddonError, AddonParam, AddonSetting, AddonSignal, Availability,
    CredentialHandle, Credentials, FADER_MAX, Invocation, ParamKind, Reading, Trigger, app_matches,
};
pub use run::run;
