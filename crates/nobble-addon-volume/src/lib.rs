//! Volume, through Windows Core Audio.
//!
//! # Why this and not Spotify's own volume
//!
//! Spotify's Web API can set Spotify's internal volume, and it needs an OAuth
//! login, a network round trip, and **Premium** — it returns 403 on a free
//! account. This needs none of them: it moves the slider Windows already keeps
//! for every application, works offline, works on a free account, and works for
//! anything that makes noise rather than only for Spotify.
//!
//! The two are not the same control and the interface should not pretend
//! otherwise. This one does not follow playback to a phone or a Connect
//! speaker; Spotify's own does. That is why the Spotify addon has its own
//! volume action rather than this one growing a mode.
//!
//! # Why a fader and not two keys
//!
//! Volume is the reason [ADR-0012] exists. A pair of up/down keys can be
//! device-resolved HID and needs no addon at all; what a fader gives is
//! *absolute* position — the control ends up where your hand is, rather than
//! wherever a count of key presses left it.
//!
//! That comes at a price the interface has to state: a fader bound here is
//! host-resolved, so it stops working when the daemon does. A fader bound to
//! MIDI keeps working with the client closed (ADR-0007). Both are legitimate
//! and they are different promises.
//!
//! # The taper
//!
//! Windows' `ISimpleAudioVolume` is a **scalar**, and scalars are linear in
//! amplitude while hearing is not. A fader mapped straight through spends its
//! top half doing almost nothing audible and its bottom tenth doing everything,
//! which feels broken long before anyone works out why.
//!
//! So the fraction is cubed. That is the same curve Windows' own volume mixer
//! applies, and it is deliberately not a decibel taper: matching the slider a
//! user already has beats being theoretically right about loudness.
//!
//! [ADR-0012]: ../../../docs/decisions/0012-addon-actions-carry-data.md

use nobble_addon_sdk::{
    Addon, AddonAction, AddonError, AddonParam, Availability, DeviceAction, DeviceKeystroke,
    Invocation, ParamKind, Trigger,
};

/// Which application's volume to move.
///
/// Optional. Absent means the **system** volume — the master slider — which is
/// the one most people reach for and the one that works even when nothing is
/// playing.
const TARGET: &[AddonParam] = &[AddonParam {
    id: "app",
    name: "Application",
    description: "Whose volume to change. Leave empty for the system volume.",
    kind: ParamKind::App,
    required: false,
    ..AddonParam::BASE
}];

/// Which applications lose the microphone.
///
/// Several, not one: silencing yourself usually means silencing the call you
/// are in *and* the thing recording it, and a single-valued parameter would
/// mean two keys for one intention.
///
/// Empty is not "nothing" — it is the microphone itself, which is the form that
/// needs no cooperation from any application and works on all of them at once.
const MIC_TARGETS: &[AddonParam] = &[AddonParam {
    id: "apps",
    name: "Applications",
    description: "Which applications lose the microphone. Leave empty to mute the microphone \
                  itself, which silences it for everything.",
    kind: ParamKind::App,
    required: false,
    multiple: true,
    ..AddonParam::BASE
}];

/// Which applications get silenced in both directions.
const DEAFEN_TARGETS: &[AddonParam] = &[AddonParam {
    id: "apps",
    name: "Applications",
    description: "Which applications to silence in both directions. Defaults to Discord.",
    kind: ParamKind::App,
    required: false,
    multiple: true,
    ..AddonParam::BASE
}];

/// What *Deafen* aims at when nothing is named.
///
/// A default rather than a required parameter, because a key that does nothing
/// until it is configured is a key that looks broken on the day it is bound.
/// It is stated in the parameter description so the interface shows it rather
/// than the user discovering it.
const DEAFEN_DEFAULT: &str = "discord";

const ACTIONS: &[AddonAction] = &[
    AddonAction {
        id: "set",
        name: "Set volume",
        description: "Moves the volume to where the fader is.",
        trigger: Trigger::Continuous,
        params: TARGET,
    },
    AddonAction {
        id: "mute",
        name: "Toggle mute",
        description: "Mutes or unmutes.",
        trigger: Trigger::Momentary,
        params: TARGET,
    },
    AddonAction {
        id: "mute_microphone",
        name: "Mute the microphone",
        description: "Mutes the microphone itself, so everything loses it at once. Name \
                      applications to take it from only those.",
        trigger: Trigger::Momentary,
        params: MIC_TARGETS,
    },
    AddonAction {
        id: "deafen",
        name: "Deafen",
        description: "Silences an application both ways at once: it loses the microphone and \
                      its own sound is muted.",
        trigger: Trigger::Momentary,
        params: DEAFEN_TARGETS,
    },
];

/// The step a user has to take in Discord, which Nobble cannot take for them.
///
/// Discord ships **no** keybind for mute or deafen — every user invents one —
/// so whatever the device sends has to be transcribed into Discord by hand.
/// This is the one dependency in the product that cannot be verified from here:
/// Nobble cannot read another application's settings, so it says so rather than
/// claiming a key works when it may not.
const IN_DISCORD: &str = "Set the matching keybind in Discord first: User Settings → Keybinds. Nobble cannot \
     check this for you, so the key will do nothing until you have.";

/// The two Discord controls the **device** can work on its own.
///
/// # Why these are here and not in the Discord addon
///
/// Because they need no Discord addon, no account and no network — the device
/// sends a keystroke and Discord's own global keybind catches it. That also
/// means they keep working with the client shut down, which nothing routed
/// through the daemon can promise.
///
/// # Why function keys and not a chord
///
/// F13 and F14 exist on no keyboard most people own, which is exactly the
/// point: nothing else is listening for them, so there is nothing to collide
/// with — no browser shortcut, no window manager, no other application's global
/// binding. A chord like Ctrl+Shift+M is guessable but already taken in several
/// places, and the failure mode of a collision is the worst kind: the key does
/// something, just not the thing it says.
///
/// They are also layout-independent. A letter's usage code is the key in that
/// *position*, so a chord chosen on one layout lands somewhere else on another;
/// a function key is the same key everywhere.
///
/// The user can change both, and has to be able to — only they know what they
/// set in Discord.
const DEVICE_ACTIONS: &[DeviceAction] = &[
    DeviceAction {
        id: "discord_toggle_mute",
        name: "Discord: toggle mute",
        description: "Mutes and unmutes you in Discord, exactly as its own keybind does — \
                      including with Nobble shut down.",
        keystroke: DeviceKeystroke::Tap {
            key: 0x68, // F13
            ctrl: false,
            shift: false,
            alt: false,
            gui: false,
        },
        prerequisite: Some(IN_DISCORD),
    },
    DeviceAction {
        id: "discord_toggle_deafen",
        name: "Discord: toggle deafen",
        description: "Deafens and undeafens you in Discord — you stop hearing the call and \
                      the call stops hearing you.",
        keystroke: DeviceKeystroke::Tap {
            key: 0x69, // F14
            ctrl: false,
            shift: false,
            alt: false,
            gui: false,
        },
        prerequisite: Some(IN_DISCORD),
    },
];

/// Windows' volume mixer, as an addon.
#[derive(Debug, Default)]
pub struct Volume {
    _private: (),
}

impl Volume {
    /// A new addon. Does not touch the platform until asked.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What the volume currently is, `0.0..=1.0`, for a named application or
    /// the system.
    #[must_use]
    pub fn level(&self, app: Option<&str>) -> Option<f32> {
        platform::level(app)
    }
}

/// Turn a fader fraction into an amplitude scalar.
///
/// Cubed, for the reason in the module documentation: a linear map spends the
/// fader's top half doing nothing audible. Kept out of `platform` so it is
/// testable without Windows, which matters because it is the one piece of this
/// addon that is a *choice* rather than a call.
#[must_use]
pub fn taper(fraction: f32) -> f32 {
    // Clamped rather than trusted. The fraction comes from a 14-bit position so
    // it cannot exceed 1.0 today, but `SetMasterVolume` rejects out-of-range
    // and would fail the whole action rather than the arithmetic.
    let f = fraction.clamp(0.0, 1.0);
    f * f * f
}

impl Addon for Volume {
    fn id(&self) -> &'static str {
        "volume"
    }

    fn name(&self) -> &'static str {
        "Volume"
    }

    fn description(&self) -> &'static str {
        "Moves the Windows volume — the system slider, or one application's. No account needed."
    }

    fn actions(&self) -> &'static [AddonAction] {
        ACTIONS
    }

    fn device_actions(&self) -> &'static [DeviceAction] {
        // Never reached by `perform`: binding one of these writes a keystroke
        // and the press goes straight from the device to Discord. That is the
        // whole point — they are the only things this addon offers that survive
        // the daemon being shut down.
        DEVICE_ACTIONS
    }

    fn availability(&self) -> Availability {
        platform::availability()
    }

    fn perform(&mut self, action: &str, invocation: &Invocation<'_>) -> Result<(), AddonError> {
        let app = invocation.param("app");
        match action {
            "set" => {
                // The trigger check in `Registry::perform` guarantees a value
                // for a continuous action, so this cannot be `None` in
                // practice. Saying so rather than unwrapping keeps the addon
                // honest if it is ever called directly.
                let Some(fraction) = invocation.fraction() else {
                    return Err(AddonError::Failed(
                        "setting the volume needs a fader position".to_owned(),
                    ));
                };
                platform::set(app, taper(fraction))
            }
            "mute" => platform::toggle_mute(app),
            "mute_microphone" => platform::toggle_microphone(&invocation.param_all("apps")),
            "deafen" => {
                let named = invocation.param_all("apps");
                let apps = if named.is_empty() {
                    vec![DEAFEN_DEFAULT]
                } else {
                    named
                };
                platform::toggle_deafen(&apps)
            }
            other => Err(AddonError::NoSuchAction(other.to_owned())),
        }
    }
}

#[cfg(windows)]
mod platform {
    use nobble_addon_sdk::{AddonError, Availability};
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, EDataFlow, ERole, IAudioSessionControl2, IAudioSessionManager2,
        IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, eCapture, eCommunications,
        eMultimedia, eRender,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
    };
    use windows::core::Interface;

    use nobble_addon_sdk::app_matches;

    /// COM initialised for the duration of one call, and undone afterwards.
    ///
    /// The signal poll and the action worker are different threads, and COM
    /// apartment state is per thread — so initialising once at startup would
    /// leave whichever thread did not do it unable to make a call. Doing it per
    /// call is a few microseconds and cannot get this wrong.
    ///
    /// `CoInitializeEx` returning `RPC_E_CHANGED_MODE` means the thread is
    /// already in an apartment, which is fine: the call still works, and only
    /// the matching `CoUninitialize` must be skipped.
    struct Com(bool);

    impl Com {
        fn enter() -> Self {
            // SAFETY: a plain COM initialisation with no pointer arguments. The
            // matching CoUninitialize is in Drop, and is skipped when this call
            // did not take ownership of the apartment.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            Self(hr.is_ok())
        }
    }

    impl Drop for Com {
        fn drop(&mut self) {
            if self.0 {
                // SAFETY: paired with the successful CoInitializeEx above, on
                // the same thread, exactly once.
                unsafe { CoUninitialize() };
            }
        }
    }

    /// One endpoint's master volume, whichever direction it faces.
    ///
    /// # The capture direction does not need a microphone permission
    ///
    /// **Measured on 2026-08-19**, Windows 11 Pro 26100-class, against the
    /// default communications capture endpoint: `Activate` succeeded,
    /// `GetMute` and `SetMute` both succeeded, no consent dialog appeared, and
    /// afterwards Windows had recorded **no** microphone use for the process —
    /// while listing twelve other applications that had genuinely captured.
    /// The privacy gate is about opening a capture *stream*; muting an
    /// endpoint is device control and is not the same operation.
    ///
    /// That is why this takes a direction rather than the capture side getting
    /// a separate, more careful implementation: there is nothing to be careful
    /// about, and one function means the two cannot drift.
    fn endpoint_for(flow: EDataFlow, role: ERole) -> Option<IAudioEndpointVolume> {
        // SAFETY: standard COM activation. Every pointer is produced by the
        // call itself and dropped by the projection's reference counting.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(flow, role).ok()?;
            device.Activate(CLSCTX_ALL, None).ok()
        }
    }

    /// The default playback device's master volume.
    fn endpoint() -> Option<IAudioEndpointVolume> {
        endpoint_for(eRender, eMultimedia)
    }

    /// The default microphone's mute control.
    ///
    /// `eCommunications` rather than `eMultimedia`: Windows lets a person
    /// nominate a different device for calls than for recording, and the one
    /// this addon is asked to mute is the one their calls are using.
    fn capture_endpoint() -> Option<IAudioEndpointVolume> {
        endpoint_for(eCapture, eCommunications)
    }

    /// One application's audio session, found by name.
    ///
    /// Matched on the process's executable rather than on the session
    /// identifier, because the session identifier is a device path with the
    /// executable buried in it and the foreground watcher speaks in executables
    /// (FR-014). The comparison is the SDK's `app_matches`, which is where it
    /// belongs: `ParamKind::App` is an SDK concept, so how to compare one has
    /// to be too, or every addon guesses separately and they disagree.
    fn app_session(app: &str) -> Option<ISimpleAudioVolume> {
        app_session_on(eRender, eMultimedia, app)
    }

    /// The named application's **capture** session, if it has one.
    ///
    /// Absent far more often than the render side, and that is the behaviour
    /// rather than a bug: an application only holds a capture session while it
    /// is actually listening. An application that is not in a call has nothing
    /// here to mute, which is why the caller has to be able to say *which*
    /// application it could not reach instead of reporting success.
    fn app_capture_session(app: &str) -> Option<ISimpleAudioVolume> {
        app_session_on(eCapture, eCommunications, app)
    }

    /// One application's session on a given endpoint.
    fn app_session_on(flow: EDataFlow, role: ERole, app: &str) -> Option<ISimpleAudioVolume> {
        // SAFETY: standard COM activation and enumeration; every interface
        // pointer comes from a call that succeeded and is reference counted by
        // the projection.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator.GetDefaultAudioEndpoint(flow, role).ok()?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).ok()?;
            let sessions = manager.GetSessionEnumerator().ok()?;
            for i in 0..sessions.GetCount().ok()? {
                let Ok(control) = sessions.GetSession(i) else {
                    continue;
                };
                let Ok(control2) = control.cast::<IAudioSessionControl2>() else {
                    continue;
                };
                // `PWSTR::to_string` is fallible here because Windows hands
                // back UTF-16 that need not be valid. A session whose name
                // cannot be decoded is skipped rather than matched loosely:
                // the alternative is aiming at whichever process happened to
                // have an odd identifier.
                let Ok(Ok(id)) = control2.GetSessionIdentifier().map(|s| s.to_string()) else {
                    continue;
                };
                if app_matches(&id, app)
                    && let Ok(volume) = control.cast::<ISimpleAudioVolume>()
                {
                    return Some(volume);
                }
            }
            None
        }
    }

    pub fn availability() -> Availability {
        let _com = Com::enter();
        match endpoint() {
            Some(_) => Availability::Ready,
            None => {
                Availability::Unavailable("Windows is not reporting a playback device.".to_owned())
            }
        }
    }

    pub fn level(app: Option<&str>) -> Option<f32> {
        let _com = Com::enter();
        // SAFETY: reads through interface pointers that were just obtained.
        unsafe {
            match app {
                Some(a) => app_session(a)?.GetMasterVolume().ok(),
                None => endpoint()?.GetMasterVolumeLevelScalar().ok(),
            }
        }
    }

    /// The named application's session, or a failure that names it.
    ///
    /// Split out because both `set` and `toggle_mute` want exactly this, and
    /// the message matters: "spotify is not playing audio" sends someone to
    /// start playback, while a generic failure sends them to the binding.
    fn require_session(app: &str) -> Result<ISimpleAudioVolume, AddonError> {
        app_session(app)
            .ok_or_else(|| AddonError::Unavailable(format!("{app} is not playing audio")))
    }

    /// The default endpoint, or a failure.
    fn require_endpoint() -> Result<IAudioEndpointVolume, AddonError> {
        endpoint().ok_or_else(|| {
            AddonError::Unavailable("Windows is not reporting a playback device".to_owned())
        })
    }

    pub fn set(app: Option<&str>, scalar: f32) -> Result<(), AddonError> {
        let _com = Com::enter();
        if let Some(a) = app {
            let session = require_session(a)?;
            // SAFETY: a scalar set on an interface just obtained. The null GUID
            // means "no event context", which is correct: nothing here is
            // listening for its own change.
            return unsafe { session.SetMasterVolume(scalar, std::ptr::null()) }
                .map_err(|e| AddonError::Failed(e.message()));
        }
        let endpoint = require_endpoint()?;
        // SAFETY: as above.
        unsafe { endpoint.SetMasterVolumeLevelScalar(scalar, std::ptr::null()) }
            .map_err(|e| AddonError::Failed(e.message()))
    }

    pub fn toggle_mute(app: Option<&str>) -> Result<(), AddonError> {
        let _com = Com::enter();
        if let Some(a) = app {
            let session = require_session(a)?;
            // SAFETY: read-then-write through one interface pointer, on one
            // thread, with no intervening call that could invalidate it.
            return unsafe {
                let muted = session
                    .GetMute()
                    .map_err(|e| AddonError::Failed(e.message()))?;
                session
                    .SetMute(!muted.as_bool(), std::ptr::null())
                    .map_err(|e| AddonError::Failed(e.message()))
            };
        }
        let endpoint = require_endpoint()?;
        // SAFETY: as above.
        unsafe {
            let muted = endpoint
                .GetMute()
                .map_err(|e| AddonError::Failed(e.message()))?;
            endpoint
                .SetMute(!muted.as_bool(), std::ptr::null())
                .map_err(|e| AddonError::Failed(e.message()))
        }
    }

    /// Flip a set of mute controls together, and say which targets were missed.
    ///
    /// # Why one state rather than each toggling itself
    ///
    /// Toggling several controls individually is not a toggle, it is a scatter:
    /// press once with two applications where one is already muted and they
    /// swap, so pressing a key twice leaves things exactly as they were and the
    /// key looks broken. So the *set* has a state — muted only when everything
    /// in it is muted — and the key moves the whole set to the other one.
    ///
    /// # Why it acts before it complains
    ///
    /// A named application that has no session cannot be silenced, and refusing
    /// to touch the rest because of it would make one absent application
    /// disable the key. The reachable ones are done, and the caller is told
    /// which were not, by name.
    fn flip_together(controls: &[ISimpleAudioVolume], missed: &[String]) -> Result<(), AddonError> {
        if !controls.is_empty() {
            // SAFETY: read-then-write through pointers obtained above, on one
            // thread, with no intervening call that could invalidate them.
            let all_muted = unsafe {
                controls
                    .iter()
                    .all(|c| c.GetMute().is_ok_and(windows::core::BOOL::as_bool))
            };
            for control in controls {
                // SAFETY: as above. The null GUID means "no event context",
                // which is correct: nothing here listens for its own change.
                unsafe { control.SetMute(!all_muted, std::ptr::null()) }
                    .map_err(|e| AddonError::Failed(e.message()))?;
            }
        }

        if missed.is_empty() {
            return Ok(());
        }
        // Naming them is the whole requirement. "Some applications could not be
        // muted" sends somebody to check all of them; this sends them to one.
        Err(AddonError::Unavailable(format!(
            "no audio session to silence for {} — an application only has one while it is \
             actually using the microphone or playing sound",
            missed.join(", ")
        )))
    }

    /// Mute the microphone itself, or the named applications' use of it.
    ///
    /// With nothing named this is the endpoint, which every application loses
    /// at once and which is the reason this needs no per-application support
    /// from anybody.
    pub fn toggle_microphone(apps: &[&str]) -> Result<(), AddonError> {
        let _com = Com::enter();
        if apps.is_empty() {
            let endpoint = capture_endpoint().ok_or_else(|| {
                AddonError::Unavailable("Windows is not reporting a microphone".to_owned())
            })?;
            // SAFETY: read-then-write through one interface pointer, on one
            // thread, with no intervening call that could invalidate it.
            return unsafe {
                let muted = endpoint
                    .GetMute()
                    .map_err(|e| AddonError::Failed(e.message()))?;
                endpoint
                    .SetMute(!muted.as_bool(), std::ptr::null())
                    .map_err(|e| AddonError::Failed(e.message()))
            };
        }

        let mut controls = Vec::new();
        let mut missed = Vec::new();
        for app in apps {
            match app_capture_session(app) {
                Some(session) => controls.push(session),
                None => missed.push((*app).to_owned()),
            }
        }
        flip_together(&controls, &missed)
    }

    /// Silence the named applications **both ways**.
    ///
    /// Deafening is not the same as muting, and doing half of it is the failure
    /// worth guarding against: an application that loses the microphone but is
    /// still audible has not been deafened, it has been muted with extra steps.
    /// So each application contributes both of its sessions, and they move
    /// together — which also makes the set-state rule above do the right thing,
    /// because a half-silenced application counts as not silenced.
    pub fn toggle_deafen(apps: &[&str]) -> Result<(), AddonError> {
        let _com = Com::enter();
        let mut controls = Vec::new();
        let mut missed = Vec::new();
        for app in apps {
            let capture = app_capture_session(app);
            let render = app_session(app);
            if capture.is_none() && render.is_none() {
                missed.push((*app).to_owned());
            }
            controls.extend(capture);
            controls.extend(render);
        }
        flip_together(&controls, &missed)
    }

    // Silences the unused-import warning on the enumeration constant, which is
    // referenced only for documentation of what `GetSessionEnumerator` returns.
    const _: u32 = DEVICE_STATE_ACTIVE.0;
}

#[cfg(not(windows))]
mod platform {
    use nobble_addon_sdk::{AddonError, Availability};

    const ONLY_WINDOWS: &str = "volume control needs Windows for now";

    pub fn availability() -> Availability {
        Availability::Unavailable(ONLY_WINDOWS.to_owned())
    }

    pub fn level(_app: Option<&str>) -> Option<f32> {
        None
    }

    pub fn set(_app: Option<&str>, _scalar: f32) -> Result<(), AddonError> {
        Err(AddonError::Unavailable(ONLY_WINDOWS.to_owned()))
    }

    pub fn toggle_mute(_app: Option<&str>) -> Result<(), AddonError> {
        Err(AddonError::Unavailable(ONLY_WINDOWS.to_owned()))
    }

    pub fn toggle_microphone(_apps: &[&str]) -> Result<(), AddonError> {
        Err(AddonError::Unavailable(ONLY_WINDOWS.to_owned()))
    }

    pub fn toggle_deafen(_apps: &[&str]) -> Result<(), AddonError> {
        Err(AddonError::Unavailable(ONLY_WINDOWS.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fader_ends_are_silence_and_full() {
        // Whatever the curve does in between, both ends have to be exact. A
        // taper that bottomed out at 0.001 would leave a fader pulled all the
        // way down still audible, which reads as broken hardware.
        assert!((taper(0.0) - 0.0).abs() < f32::EPSILON);
        assert!((taper(1.0) - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn the_taper_is_not_linear() {
        // The whole point. Halfway up the fader is a quarter of the amplitude,
        // which is roughly where a person expects "half volume" to sit --
        // linear would put it at 0.5 and sound almost as loud as the top.
        let half = taper(0.5);
        assert!(half < 0.2, "halfway should be quiet, got {half}");
        assert!(half > 0.1, "and not silent, got {half}");
    }

    #[test]
    fn the_taper_never_goes_backwards() {
        // A fader that got quieter as it was pushed up would be the funniest
        // possible bug and the hardest to believe from a report.
        let mut last = -1.0;
        for step in 0..=100 {
            #[allow(clippy::cast_precision_loss)]
            let now = taper(step as f32 / 100.0);
            assert!(now >= last, "went backwards at {step}");
            last = now;
        }
    }

    #[test]
    fn an_impossible_fraction_is_clamped_rather_than_passed_on() {
        // `SetMasterVolume` rejects out of range, so an unclamped value would
        // fail the whole action rather than the arithmetic -- and the user
        // would see "the fader does nothing" with no clue why.
        assert!((taper(1.5) - 1.0).abs() < f32::EPSILON);
        assert!((taper(-0.5) - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn setting_the_volume_wants_a_fader_and_muting_wants_a_key() {
        // The distinction ADR-0012 introduced, on the addon it was introduced
        // for. Getting these the wrong way round would let someone bind mute to
        // a fader, where a sweep would toggle it a hundred times.
        let set = ACTIONS.iter().find(|a| a.id == "set").unwrap();
        let mute = ACTIONS.iter().find(|a| a.id == "mute").unwrap();
        assert_eq!(set.trigger, Trigger::Continuous);
        assert_eq!(mute.trigger, Trigger::Momentary);
    }

    #[test]
    fn the_target_is_optional_so_the_system_slider_is_the_default() {
        // Found by kind rather than by id, because the aiming parameter is
        // `app` on the actions that take one and `apps` on the ones that take
        // several. What has to hold for all of them is that aiming is
        // *optional* — every one of these does something sensible unbound.
        for a in ACTIONS {
            let target = a
                .params
                .iter()
                .find(|p| p.kind == ParamKind::App)
                .unwrap_or_else(|| panic!("{} cannot be aimed at anything", a.id));
            assert!(
                !target.required,
                "{} does nothing until it is configured",
                a.id
            );
        }
    }

    #[test]
    fn silencing_takes_several_applications_and_moving_a_slider_takes_one() {
        // A fader has one position and cannot send it to two places, so `set`
        // stays single. Silencing yourself usually means silencing the call
        // *and* whatever is recording it, which is one intention and should not
        // need two keys.
        for id in ["mute_microphone", "deafen"] {
            let action = ACTIONS.iter().find(|a| a.id == id).expect(id);
            assert!(
                action.param("apps").expect("apps").multiple,
                "{id} must accept a list"
            );
        }
        assert!(
            !ACTIONS
                .iter()
                .find(|a| a.id == "set")
                .expect("set")
                .param("app")
                .expect("app")
                .multiple
        );
    }

    #[test]
    fn every_action_is_triggered_by_the_kind_of_input_that_suits_it() {
        // Silencing is a key, not a fader: a sweep across a continuous binding
        // would toggle it once per step.
        for id in ["mute_microphone", "deafen"] {
            let action = ACTIONS.iter().find(|a| a.id == id).expect(id);
            assert_eq!(action.trigger, Trigger::Momentary, "{id}");
        }
    }

    #[test]
    fn deafen_names_its_default_where_a_user_will_read_it() {
        // The default is in code, so the only thing stopping it being a secret
        // is the description. A key that silently aims somewhere is worse than
        // one that does nothing.
        let deafen = ACTIONS.iter().find(|a| a.id == "deafen").expect("deafen");
        let apps = deafen.param("apps").expect("apps");
        assert!(
            apps.description.to_lowercase().contains(DEAFEN_DEFAULT),
            "the default target is not stated: {}",
            apps.description
        );
    }

    #[test]
    fn the_device_resolved_pair_admits_it_needs_a_step_elsewhere() {
        // Discord ships no keybind for either, so the declared keystroke is a
        // suggestion the user has to transcribe. `Some` obliges the interface
        // not to claim the key works — a blank string would make "the author
        // left it empty" and "there is nothing to do" the same value.
        assert_eq!(DEVICE_ACTIONS.len(), 2);
        for action in DEVICE_ACTIONS {
            let note = action
                .prerequisite
                .unwrap_or_else(|| panic!("{} claims to need nothing", action.id));
            assert!(
                note.contains("Keybinds"),
                "{} does not say where to go: {note}",
                action.id
            );
        }
    }

    #[test]
    fn the_suggested_keys_collide_with_nothing_and_carry_no_modifiers() {
        // F13 and F14. Nothing else listens for them, and a function key is the
        // same physical key on every layout — unlike a letter, whose usage code
        // is a *position* and lands elsewhere on AZERTY.
        //
        // A modifier here would be a second thing to get right in Discord for
        // no benefit, since the whole reason these keys were chosen is that
        // they are already unique.
        let keys: Vec<u8> = DEVICE_ACTIONS
            .iter()
            .map(|a| match a.keystroke {
                DeviceKeystroke::Tap {
                    key,
                    ctrl,
                    shift,
                    alt,
                    gui,
                } => {
                    assert!(!ctrl && !shift && !alt && !gui, "{} holds a modifier", a.id);
                    key
                }
                DeviceKeystroke::Consumer { .. } => {
                    panic!("{} is not a media key", a.id)
                }
            })
            .collect();
        assert_eq!(keys, vec![0x68, 0x69], "F13 and F14");
    }

    #[test]
    fn a_device_action_never_shares_an_id_with_one_the_daemon_performs() {
        // They are bound through the same list and stored the same way, so a
        // collision would make a binding ambiguous — and the one that lost
        // would fail in the least explicable way available: a key that types a
        // keystroke when it was supposed to call the addon, or the reverse.
        for device in DEVICE_ACTIONS {
            assert!(
                !ACTIONS.iter().any(|a| a.id == device.id),
                "{} is declared twice",
                device.id
            );
        }
    }

    #[test]
    fn an_unknown_action_is_refused() {
        let mut addon = Volume::new();
        assert!(matches!(
            addon.perform("teleport", &Invocation::bare()),
            Err(AddonError::NoSuchAction(_))
        ));
    }
}
