//! The loop an addon does not have to write.
//!
//! [`run`] takes your [`Addon`] and serves the protocol on stdin and stdout
//! until the daemon says stop. You never see a frame.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::sync::{Arc, Mutex};

use crate::addon::{
    Addon, AddonAction, AddonChoices, AddonError, AddonParam, AddonSetting, AddonSignal,
    Availability, CredentialHandle, Credentials, DeviceAction, DeviceKeystroke, Invocation,
    Permission, Reading,
};
use crate::protocol::{
    ActionDecl, Answer, Ask, AvailabilityDecl, ChoiceDecl, ChoicesDecl, Description,
    DeviceActionDecl, FailureKind, KeystrokeDecl, PROTOCOL, ParamDecl, PermissionDecl, ReadingDecl,
    Reply, Request, SettingDecl, SignalDecl,
};

/// Serve an addon on stdin and stdout, until told to stop.
///
/// Returns when the daemon sends `Shutdown` or closes the pipe. An addon's
/// `main` is normally nothing but this.
pub fn run<A: Addon>(addon: A) {
    let stdin = BufReader::new(std::io::stdin());
    let stdout = std::io::stdout();
    // A broken pipe means the daemon went away, which is its business and not
    // an error to shout about: exit quietly and let it restart us.
    let _ = serve(addon, stdin, stdout);
}

/// [`run`], with the pipe supplied. For tests, and for anything hosting an
/// addon over something other than stdio.
///
/// # Errors
/// If the pipe fails. A malformed line is answered with a failure rather than
/// returned — one bad frame should not end the conversation.
pub fn serve<A: Addon, R: BufRead + Send + 'static, W: Write + Send + 'static>(
    mut addon: A,
    reader: R,
    writer: W,
) -> std::io::Result<()> {
    let pipe = Arc::new(Mutex::new(Pipe {
        reader,
        writer,
        line: String::new(),
    }));
    let credentials: CredentialHandle = Arc::new(WireCredentials {
        pipe: Arc::clone(&pipe),
    });

    loop {
        let Some(text) = read_line(&pipe)? else {
            return Ok(()); // the daemon closed the pipe
        };
        let request: Request = match serde_json::from_str(&text) {
            Ok(r) => r,
            Err(e) => {
                // Answered rather than fatal. A frame this end cannot parse is
                // a bug in one message, and killing the conversation over it
                // would turn a bad request into a dead addon.
                send(
                    &pipe,
                    &Reply::Failed {
                        kind: FailureKind::Failed,
                        detail: format!("could not understand that: {e}"),
                    },
                )?;
                continue;
            }
        };

        let reply = match request {
            // Version checking is the daemon's call, not ours: it is the one that
            // can report a mismatch to a user and name the fix (FR-049). Answering
            // honestly and letting it decide is the whole of our part.
            Request::Hello { .. } => Reply::Welcome { version: PROTOCOL },
            Request::Describe => Reply::Description(describe(&addon)),
            Request::Availability => Reply::Availability(match addon.availability() {
                Availability::Ready => AvailabilityDecl::Ready,
                Availability::Unavailable(detail) => AvailabilityDecl::Unavailable { detail },
            }),
            Request::Applies { action } => Reply::Applies {
                applies: addon.applies(&action),
            },
            Request::Status => Reply::Status {
                status: addon.status(),
            },
            Request::LiveChoices { id } => Reply::LiveChoices {
                choices: addon
                    .live_choices(&id)
                    .into_iter()
                    .map(|c| ChoiceDecl {
                        value: c.value,
                        label: c.label,
                        detail: c.detail,
                    })
                    .collect(),
            },
            Request::BoundInputs { action, inputs } => {
                addon.bound_inputs(&action, &inputs);
                Reply::Done
            }
            Request::Configure { values } => {
                addon.configure(&values, Arc::clone(&credentials));
                Reply::Done
            }
            Request::ReadSignals => Reply::Signals(match addon.read_signals() {
                Reading::Values(values) => ReadingDecl::Values {
                    values: values
                        .into_iter()
                        .map(|(id, on)| (id.to_owned(), on))
                        .collect(),
                },
                Reading::Unavailable(detail) => ReadingDecl::Unavailable { detail },
            }),
            Request::Perform {
                action,
                params,
                value,
            } => perform(&mut addon, &action, &params, value),
            Request::Shutdown => {
                send(&pipe, &Reply::Done)?;
                return Ok(());
            }
        };
        send(&pipe, &reply)?;
    }
}

fn perform<A: Addon>(
    addon: &mut A,
    action: &str,
    params: &BTreeMap<String, crate::addon::ParamValue>,
    value: Option<u16>,
) -> Reply {
    // ADR-0021's backward-compatibility half. A daemon that understands
    // `device_actions` resolved this to a keystroke when the key was bound and
    // never sends it here at all; a daemon built against SDK 1.0 dropped the
    // field on the way in, because serde discards what it does not recognise,
    // and will ask for a `perform` the author was told not to write.
    //
    // Answered here rather than left to fall through, because the fall-through
    // is `AddonError::NoSuchAction` and that sentence is a lie: the action is
    // right there in the list the user bound it from, and they would go looking
    // for a missing action instead of an old Nobble.
    //
    // `Failed` rather than `Unavailable`: unavailable means try again later, and
    // this never will be until the daemon is replaced.
    if addon.device_actions().iter().any(|d| d.id == action) {
        return Reply::Failed {
            kind: FailureKind::Failed,
            detail: format!(
                "{action:?} is sent by the device, not by this addon. \
                 This version of Nobble is older than the addon and cannot bind \
                 it as a keystroke; update Nobble."
            ),
        };
    }

    let invocation = match value {
        Some(v) => Invocation::moved(params, v),
        None => Invocation::press(params),
    };
    match addon.perform(action, &invocation) {
        Ok(()) => Reply::Done,
        Err(e) => {
            let (kind, detail) = match e {
                AddonError::NoSuchAction(w) => (FailureKind::NoSuchAction, w),
                AddonError::Unavailable(w) => (FailureKind::Unavailable, w),
                AddonError::Failed(w) => (FailureKind::Failed, w),
            };
            Reply::Failed { kind, detail }
        }
    }
}

/// Derive the owned declaration from the borrowed one the author wrote.
///
/// This function is why `protocol`'s mirrors are not a second definition to
/// keep in step: there is one authored source, and this is the only thing that
/// produces the other.
fn describe<A: Addon>(addon: &A) -> Description {
    Description {
        id: addon.id().to_owned(),
        name: addon.name().to_owned(),
        description: addon.description().to_owned(),
        actions: addon.actions().iter().map(action_decl).collect(),
        device_actions: addon
            .device_actions()
            .iter()
            .map(device_action_decl)
            .collect(),
        choices: addon.choices().iter().map(choices_decl).collect(),
        signals: addon.signals().iter().map(signal_decl).collect(),
        settings: addon.settings().iter().map(setting_decl).collect(),
        permissions: addon.permissions().iter().map(permission_decl).collect(),
    }
}

fn permission_decl(p: &Permission) -> PermissionDecl {
    match *p {
        Permission::Network { host, reason } => PermissionDecl::Network {
            host: host.to_owned(),
            reason: reason.to_owned(),
        },
        Permission::Files {
            path,
            write,
            reason,
        } => PermissionDecl::Files {
            path: path.to_owned(),
            write,
            reason: reason.to_owned(),
        },
        Permission::Launch { program, reason } => PermissionDecl::Launch {
            program: program.to_owned(),
            reason: reason.to_owned(),
        },
        Permission::Credentials { reason } => PermissionDecl::Credentials {
            reason: reason.to_owned(),
        },
    }
}

fn action_decl(a: &AddonAction) -> ActionDecl {
    ActionDecl {
        id: a.id.to_owned(),
        name: a.name.to_owned(),
        description: a.description.to_owned(),
        trigger: a.trigger.as_wire().to_owned(),
        params: a.params.iter().map(param_decl).collect(),
    }
}

/// The device-resolved half of the same one-way derivation.
///
/// A second function beside [`action_decl`] rather than a branch inside it,
/// because the two produce different shapes — this one has no trigger and no
/// parameters to convert. It is still the *only* thing that produces a
/// [`DeviceActionDecl`], which is the property that matters: one authored
/// source, one derivation per owned type, nothing hand-maintaining the mirror.
fn device_action_decl(d: &DeviceAction) -> DeviceActionDecl {
    DeviceActionDecl {
        id: d.id.to_owned(),
        name: d.name.to_owned(),
        description: d.description.to_owned(),
        keystroke: keystroke_decl(d.keystroke),
        prerequisite: d.prerequisite.map(ToOwned::to_owned),
    }
}

/// Exhaustive on purpose: [`KeystrokeDecl::Unknown`] is a decode state and has
/// no arm here, so a new [`DeviceKeystroke`] variant fails to compile until
/// somebody decides what crosses the pipe for it.
fn keystroke_decl(k: DeviceKeystroke) -> KeystrokeDecl {
    match k {
        DeviceKeystroke::Tap {
            key,
            ctrl,
            shift,
            alt,
            gui,
        } => KeystrokeDecl::HidTap {
            key,
            ctrl,
            shift,
            alt,
            gui,
        },
        DeviceKeystroke::Consumer { usage } => KeystrokeDecl::HidConsumer { usage },
    }
}

fn param_decl(p: &AddonParam) -> ParamDecl {
    ParamDecl {
        id: p.id.to_owned(),
        name: p.name.to_owned(),
        description: p.description.to_owned(),
        kind: p.kind.as_wire().to_owned(),
        required: p.required,
        multiple: p.multiple,
        choices: p.choices.map(ToOwned::to_owned),
    }
}

fn choices_decl(c: &AddonChoices) -> ChoicesDecl {
    ChoicesDecl {
        id: c.id.to_owned(),
        name: c.name.to_owned(),
        live: c.live,
        values: c
            .values
            .iter()
            .map(|v| ChoiceDecl {
                value: v.value.to_owned(),
                label: v.label.to_owned(),
                // A declared source has nothing to disambiguate: its values are
                // fixed at build time, so an author who needs two of them told
                // apart writes it into the label.
                detail: String::new(),
            })
            .collect(),
    }
}

fn signal_decl(s: &AddonSignal) -> SignalDecl {
    SignalDecl {
        id: s.id.to_owned(),
        name: s.name.to_owned(),
        description: s.description.to_owned(),
    }
}

fn setting_decl(s: &AddonSetting) -> SettingDecl {
    SettingDecl {
        param: param_decl(&s.param),
        secret: s.secret,
    }
}

/// The pipe, and the buffer reads reuse.
struct Pipe<R, W> {
    reader: R,
    writer: W,
    line: String,
}

fn read_line<R: BufRead, W: Write>(
    pipe: &Arc<Mutex<Pipe<R, W>>>,
) -> std::io::Result<Option<String>> {
    let mut p = pipe
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    p.line.clear();
    let Pipe { reader, line, .. } = &mut *p;
    if reader.read_line(line)? == 0 {
        return Ok(None);
    }
    Ok(Some(p.line.trim_end().to_owned()))
}

fn send<R: BufRead, W: Write, T: serde::Serialize>(
    pipe: &Arc<Mutex<Pipe<R, W>>>,
    message: &T,
) -> std::io::Result<()> {
    let mut p = pipe
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = serde_json::to_string(message)?;
    writeln!(p.writer, "{text}")?;
    p.writer.flush()
}

/// [`Credentials`] over the pipe.
///
/// Every call is an [`Ask`] out and an [`Answer`] back, taken while the addon
/// is part-way through a request. That works because the conversation is
/// strictly nested — the daemon is waiting for our reply, so the next thing it
/// sends is the answer to this — and it is what keeps `Credentials` a plain
/// synchronous call in the addon's own code.
struct WireCredentials<R, W> {
    pipe: Arc<Mutex<Pipe<R, W>>>,
}

impl<R: BufRead + Send, W: Write + Send> WireCredentials<R, W> {
    fn exchange(&self, ask: &Ask) -> Option<Answer> {
        send(&self.pipe, &Reply::Ask(ask.clone())).ok()?;
        let text = read_line(&self.pipe).ok()??;
        serde_json::from_str(&text).ok()
    }
}

impl<R: BufRead + Send, W: Write + Send> Credentials for WireCredentials<R, W> {
    fn get(&self, key: &str) -> Option<String> {
        match self.exchange(&Ask::Get {
            key: key.to_owned(),
        })? {
            Answer::Value { value } => value,
            // A refusal and an absence are the same to the caller: there is no
            // credential to use. The daemon has already told the user why.
            Answer::Stored | Answer::Refused { .. } => None,
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        match self.exchange(&Ask::Set {
            key: key.to_owned(),
            value: value.to_owned(),
        }) {
            Some(Answer::Stored) => Ok(()),
            Some(Answer::Refused { detail }) => Err(detail),
            Some(Answer::Value { .. }) | None => Err("the daemon did not answer".to_owned()),
        }
    }

    fn clear(&self, key: &str) {
        let _ = self.exchange(&Ask::Clear {
            key: key.to_owned(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::addon::{AddonAction, Availability, Trigger};
    use std::io::Cursor;

    /// A writer the test can still read after `serve` has taken it.
    #[derive(Clone, Default)]
    struct Recorder(Arc<Mutex<Vec<u8>>>);

    impl Write for Recorder {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl Recorder {
        fn lines(&self) -> Vec<String> {
            let bytes = self
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone();
            String::from_utf8(bytes)
                .expect("utf-8")
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    const ACTIONS: &[AddonAction] = &[AddonAction {
        id: "wave",
        name: "Wave",
        description: "Says hello.",
        trigger: Trigger::Momentary,
        params: &[],
    }];

    const DEVICE_ACTIONS: &[DeviceAction] = &[DeviceAction {
        id: "salute",
        name: "Salute",
        description: "The device sends this one.",
        keystroke: DeviceKeystroke::Tap {
            key: 0x10,
            ctrl: true,
            shift: true,
            alt: false,
            gui: false,
        },
        prerequisite: Some("Set this keybind in the other application."),
    }];

    #[derive(Default)]
    struct Probe;

    impl Addon for Probe {
        fn id(&self) -> &'static str {
            "probe"
        }
        fn name(&self) -> &'static str {
            "Probe"
        }
        fn description(&self) -> &'static str {
            "For tests."
        }
        fn actions(&self) -> &'static [AddonAction] {
            ACTIONS
        }
        fn device_actions(&self) -> &'static [DeviceAction] {
            DEVICE_ACTIONS
        }
        fn availability(&self) -> Availability {
            Availability::Ready
        }
        fn perform(&mut self, action: &str, _i: &Invocation<'_>) -> Result<(), AddonError> {
            match action {
                "wave" => Ok(()),
                other => Err(AddonError::NoSuchAction(other.to_owned())),
            }
        }
        fn configure(&mut self, _values: &BTreeMap<String, String>, c: CredentialHandle) {
            // Asks mid-request on purpose: that is the nested exchange under
            // test. What it reads is asserted on the wire rather than kept in a
            // field, because the wire is the thing this crate publishes.
            let _ = c.get("token");
        }
    }

    fn script(lines: &[&str]) -> Cursor<Vec<u8>> {
        Cursor::new(lines.join("\n").into_bytes())
    }

    #[test]
    fn it_answers_a_handshake_and_describes_itself() {
        let out = Recorder::default();
        serve(
            Probe,
            script(&[
                r#"{"ask":"hello","version":{"major":1,"minor":0}}"#,
                r#"{"ask":"describe"}"#,
            ]),
            out.clone(),
        )
        .expect("served");

        let lines = out.lines();
        assert_eq!(
            lines[0],
            r#"{"say":"welcome","version":{"major":1,"minor":2}}"#
        );
        let described: Reply = serde_json::from_str(&lines[1]).expect("parse");
        let Reply::Description(d) = described else {
            panic!("expected a description, got {:?}", lines[1]);
        };
        assert_eq!(d.id, "probe");
        assert_eq!(d.actions.len(), 1);
        assert_eq!(d.actions[0].trigger, "momentary");
        // Derived from the borrowed declaration, never hand-written.
        assert_eq!(d.actions[0].id, "wave");
    }

    #[test]
    fn a_failure_keeps_its_kind() {
        // The daemon distinguishes "no such action" from "not right now"
        // without parsing prose, because US9 §4 treats them differently: one is
        // a binding to preserve and report, the other is a temporary state.
        let out = Recorder::default();
        serve(
            Probe,
            script(&[r#"{"ask":"perform","action":"nope"}"#]),
            out.clone(),
        )
        .expect("served");

        let reply: Reply = serde_json::from_str(&out.lines()[0]).expect("parse");
        assert_eq!(
            reply,
            Reply::Failed {
                kind: FailureKind::NoSuchAction,
                detail: "nope".to_owned(),
            }
        );
    }

    #[test]
    fn a_credential_is_fetched_mid_request_and_names_no_addon() {
        // The nested exchange: the daemon asks us to configure, we ask it for a
        // credential while doing so, it answers, and only then do we reply. It
        // works because the conversation is strictly nested -- the daemon is
        // already waiting on us, so the next line it sends is our answer.
        let out = Recorder::default();
        serve(
            Probe,
            script(&[
                r#"{"ask":"configure"}"#,
                r#"{"answer":"value","value":"sekrit"}"#,
            ]),
            out.clone(),
        )
        .expect("served");

        let lines = out.lines();
        assert_eq!(
            lines[0], r#"{"say":"ask","want":"get","key":"token"}"#,
            "the ask goes out first, and carries no addon name"
        );
        assert!(
            !lines[0].contains("probe"),
            "an addon must not be able to name whose credential it wants"
        );
        assert_eq!(lines[1], r#"{"say":"done"}"#, "then the reply");
    }

    /// ADR-0021's backward-compatibility half, which nothing else can test:
    /// only an *old* daemon sends this, and there is no old daemon to run
    /// against. The failure has to say why, because the honest-looking answer —
    /// `NoSuchAction` — would send the user hunting for a missing action that
    /// is right there in the list they bound it from.
    #[test]
    fn an_old_daemon_asking_us_to_perform_a_device_action_is_told_why_not() {
        let out = Recorder::default();
        serve(
            Probe,
            script(&[r#"{"ask":"perform","action":"salute"}"#]),
            out.clone(),
        )
        .expect("served");

        let Reply::Failed { kind, detail } = serde_json::from_str(&out.lines()[0]).expect("parse")
        else {
            panic!("expected a failure, got {:?}", out.lines()[0]);
        };
        // Not `Unavailable`: unavailable means try again later, and this will
        // not change until the daemon is replaced.
        assert_eq!(kind, FailureKind::Failed);
        assert!(detail.contains("sent by the device"), "{detail}");
        assert!(detail.contains("update Nobble"), "{detail}");
    }

    /// The addon author writes no `perform` arm for a device-resolved action,
    /// so the id must never reach one. `Probe::perform` answers `NoSuchAction`
    /// for anything but `wave`; if the guard above ever moves below the call,
    /// this is what notices.
    #[test]
    fn a_device_action_never_reaches_the_addons_own_perform() {
        let out = Recorder::default();
        serve(
            Probe,
            script(&[r#"{"ask":"perform","action":"salute"}"#]),
            out.clone(),
        )
        .expect("served");
        assert!(
            !out.lines()[0].contains("no_such_action"),
            "the guard let it through: {}",
            out.lines()[0]
        );
    }

    #[test]
    fn a_frame_it_cannot_parse_is_answered_rather_than_fatal() {
        // One bad message should not end the conversation, or a daemon bug
        // becomes a dead addon.
        let out = Recorder::default();
        serve(
            Probe,
            script(&["not json at all", r#"{"ask":"status"}"#]),
            out.clone(),
        )
        .expect("served");

        let lines = out.lines();
        assert!(lines[0].contains(r#""say":"failed""#), "{}", lines[0]);
        assert_eq!(
            lines[1], r#"{"say":"status","status":null}"#,
            "and it carries on"
        );
    }

    #[test]
    fn shutdown_ends_it() {
        let out = Recorder::default();
        serve(
            Probe,
            script(&[r#"{"ask":"shutdown"}"#, r#"{"ask":"status"}"#]),
            out.clone(),
        )
        .expect("served");
        assert_eq!(
            out.lines().len(),
            1,
            "nothing after shutdown is answered: {:?}",
            out.lines()
        );
    }
}
