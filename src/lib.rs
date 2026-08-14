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
    Addon, AddonAction, AddonError, AddonParam, Availability, Invocation, ParamKind, Trigger,
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
}];

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
            other => Err(AddonError::NoSuchAction(other.to_owned())),
        }
    }
}

#[cfg(windows)]
mod platform {
    use nobble_addon_sdk::{AddonError, Availability};
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        DEVICE_STATE_ACTIVE, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
        ISimpleAudioVolume, MMDeviceEnumerator, eMultimedia, eRender,
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

    /// The default playback device's master volume.
    fn endpoint() -> Option<IAudioEndpointVolume> {
        // SAFETY: standard COM activation. Every pointer is produced by the
        // call itself and dropped by the projection's reference counting.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .ok()?;
            device.Activate(CLSCTX_ALL, None).ok()
        }
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
        // SAFETY: standard COM activation and enumeration; every interface
        // pointer comes from a call that succeeded and is reference counted by
        // the projection.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, eMultimedia)
                .ok()?;
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
        for a in ACTIONS {
            let target = a.param("app").expect("every action can be aimed");
            assert!(!target.required);
            assert_eq!(target.kind, ParamKind::App);
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
