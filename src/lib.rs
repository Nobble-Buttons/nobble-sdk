//! Transport control through the Windows media session.
//!
//! # Why this and not the Spotify Web API
//!
//! Spotify's Web API can do far more — playlists, saving a track, moving
//! playback between devices — and it needs a registered application, an OAuth
//! login per user, and a refresh token the daemon has to keep. None of that
//! works while the network is down, and all of it fails in ways a user has to
//! understand.
//!
//! `GlobalSystemMediaTransportControlsSessionManager` is what Windows itself
//! uses for the media overlay. It needs no account, no client id and no
//! network, it reports the track that is actually playing, and it works for
//! whatever is playing rather than only Spotify.
//!
//! It is deliberately the *first* addon and not the only one: a Web API addon
//! is worth having for what this cannot reach, and the two can coexist because
//! a binding names the addon it wants.
//!
//! # Why not just bind a media key
//!
//! For play/pause alone, `hid_consumer` is better — the device sends it, so it
//! works with Nobble closed (ADR-0007), and this addon does not. What the media
//! session adds is *knowing*: which application is playing, and what the track
//! is called. That is what a key with a display is for, and a keyboard cannot
//! find it out alone.

use nobble_addon_sdk::{
    Addon, AddonAction, AddonError, AddonParam, AddonSignal, Availability, Invocation, ParamKind,
    Reading, Trigger,
};

/// Which application a key controls.
///
/// Optional, and absent is the useful default: "whatever is playing" is what
/// most people want from a play/pause key most of the time, and it is the
/// behaviour this addon had before parameters existed.
///
/// Naming one turns the key into a control for *that* application, which keeps
/// working when a browser tab starts a video and takes the current session
/// away. Before this, a "skip track" key would skip the video instead — a key
/// that does the wrong thing rather than nothing, which is worse.
const TARGET: &[AddonParam] = &[AddonParam {
    id: "app",
    name: "Application",
    description: "Which application to control. Leave empty for whatever is playing.",
    kind: ParamKind::App,
    required: false,
}];

const ACTIONS: &[AddonAction] = &[
    AddonAction {
        id: "play_pause",
        name: "Play / pause",
        description: "Toggles playback.",
        trigger: Trigger::Momentary,
        params: TARGET,
    },
    AddonAction {
        id: "next",
        name: "Next track",
        description: "Skips forward.",
        trigger: Trigger::Momentary,
        params: TARGET,
    },
    AddonAction {
        id: "previous",
        name: "Previous track",
        description: "Skips back.",
        trigger: Trigger::Momentary,
        params: TARGET,
    },
    AddonAction {
        id: "stop",
        name: "Stop",
        description: "Stops playback.",
        trigger: Trigger::Momentary,
        params: TARGET,
    },
];

const SIGNALS: &[AddonSignal] = &[AddonSignal {
    id: "playing",
    name: "Playing",
    description: "Something is playing — any application Windows knows about.",
}];

/// What is playing, if anything.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NowPlaying {
    /// The application that owns the session, e.g. `Spotify.exe`.
    pub source: String,
    /// Track title, empty if the session does not report one.
    pub title: String,
    /// Artist, empty if unknown.
    pub artist: String,
    /// Whether it is playing rather than paused.
    pub playing: bool,
}

/// Media transport control, through whatever Windows says is playing.
#[derive(Debug, Default)]
pub struct MediaSession {
    _private: (),
}

impl MediaSession {
    /// A new addon. Does not touch the platform until asked.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// What is playing right now, if anything.
    #[must_use]
    pub fn now_playing(&self) -> Option<NowPlaying> {
        platform::now_playing(None)
    }

    /// What a named application is playing, if it is.
    ///
    /// `app` is an application identity as the foreground watcher reports it,
    /// such as `exe:spotify.exe`.
    #[must_use]
    pub fn now_playing_for(&self, app: &str) -> Option<NowPlaying> {
        platform::now_playing(Some(app))
    }
}

impl Addon for MediaSession {
    fn id(&self) -> &'static str {
        "media"
    }

    fn name(&self) -> &'static str {
        "Media"
    }

    fn description(&self) -> &'static str {
        "Controls whatever is playing — Spotify, a browser tab, anything that registers with Windows. No account needed."
    }

    fn actions(&self) -> &'static [AddonAction] {
        ACTIONS
    }

    fn signals(&self) -> &'static [AddonSignal] {
        SIGNALS
    }

    fn availability(&self) -> Availability {
        platform::availability()
    }

    fn perform(&mut self, action: &str, invocation: &Invocation<'_>) -> Result<(), AddonError> {
        if !ACTIONS.iter().any(|a| a.id == action) {
            return Err(AddonError::NoSuchAction(action.to_owned()));
        }
        platform::perform(action, invocation.param("app"))
    }

    fn read_signals(&mut self) -> Reading {
        match platform::now_playing(None) {
            Some(n) => Reading::Values(vec![("playing", n.playing)]),
            None => Reading::Unavailable("nothing is playing".to_owned()),
        }
    }
}

#[cfg(windows)]
mod platform {
    use nobble_addon_sdk::{AddonError, Availability};
    // `join()` blocks until the WinRT operation completes. It runs on the
    // host-action worker thread, never the input path — FR-022 exists so a
    // slow action cannot delay the next keypress.
    use windows::Media::Control::GlobalSystemMediaTransportControlsSession as Session;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionManager as Manager;
    use windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status;

    use super::NowPlaying;
    use nobble_addon_sdk::app_matches;

    /// The session a binding meant, or nothing.
    ///
    /// With no target this is `GetCurrentSession` — whatever last made noise,
    /// which is the right default for a bare play/pause key. With one it walks
    /// every session and picks the match, which is what makes a key keep
    /// controlling Spotify while a browser tab plays over the top.
    ///
    /// Every call re-requests the manager rather than caching one. The cached
    /// object goes stale when the playing application changes, and the failure
    /// is silent — the next command goes to an application that has closed.
    /// This costs a COM call on a worker thread, which is not on the input
    /// path.
    fn session(target: Option<&str>) -> Option<Session> {
        let manager = Manager::RequestAsync().ok()?.join().ok()?;
        let Some(app) = target else {
            return manager.GetCurrentSession().ok();
        };
        manager.GetSessions().ok()?.into_iter().find(|s| {
            s.SourceAppUserModelId()
                .is_ok_and(|id| app_matches(&id.to_string(), app))
        })
    }

    pub fn availability() -> Availability {
        match session(None) {
            Some(_) => Availability::Ready,
            None => Availability::Unavailable(
                "Nothing is playing. Start Spotify, or anything else that reports to Windows."
                    .to_owned(),
            ),
        }
    }

    pub fn now_playing(target: Option<&str>) -> Option<NowPlaying> {
        let session = session(target)?;
        let properties = session.TryGetMediaPropertiesAsync().ok()?.join().ok()?;
        let playing = session
            .GetPlaybackInfo()
            .ok()
            .and_then(|i| i.PlaybackStatus().ok())
            .is_some_and(|s| s == Status::Playing);
        Some(NowPlaying {
            source: session
                .SourceAppUserModelId()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            title: properties
                .Title()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            artist: properties
                .Artist()
                .map(|s| s.to_string())
                .unwrap_or_default(),
            playing,
        })
    }

    pub fn perform(action: &str, target: Option<&str>) -> Result<(), AddonError> {
        let Some(session) = session(target) else {
            // Names the application when one was asked for. "Nothing is
            // playing" is a confusing thing to read when Spotify is plainly
            // open and the key was bound to Chrome.
            return Err(AddonError::Unavailable(match target {
                Some(app) => format!("{app} is not playing anything Nobble can control"),
                None => "nothing is playing for Nobble to control".to_owned(),
            }));
        };
        // Each of these returns whether the session accepted it. A `false` is
        // not an error from the API's point of view but it is one from the
        // user's: the key did nothing.
        let accepted = match action {
            "play_pause" => session.TryTogglePlayPauseAsync().and_then(|op| op.join()),
            "next" => session.TrySkipNextAsync().and_then(|op| op.join()),
            "previous" => session.TrySkipPreviousAsync().and_then(|op| op.join()),
            "stop" => session.TryStopAsync().and_then(|op| op.join()),
            other => return Err(AddonError::NoSuchAction(other.to_owned())),
        };
        match accepted {
            Ok(true) => Ok(()),
            Ok(false) => Err(AddonError::Failed(format!(
                "the application playing refused {action}"
            ))),
            Err(e) => Err(AddonError::Failed(e.message())),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use nobble_addon_sdk::{AddonError, Availability};

    use super::NowPlaying;

    pub fn availability() -> Availability {
        Availability::Unavailable("media control needs Windows for now".to_owned())
    }

    pub fn now_playing(_target: Option<&str>) -> Option<NowPlaying> {
        None
    }

    pub fn perform(_action: &str, _target: Option<&str>) -> Result<(), AddonError> {
        Err(AddonError::Unavailable(
            "media control needs Windows for now".to_owned(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_action_has_a_stable_id_and_something_to_read() {
        // Ids are stored in bindings, so a rename breaks someone's key
        // silently. Names and descriptions are what a menu shows; an empty one
        // is a blank row.
        for a in ACTIONS {
            assert!(!a.id.is_empty());
            assert!(!a.name.is_empty());
            assert!(!a.description.is_empty());
            assert!(
                a.id.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "{:?} should be a stable snake_case id",
                a.id
            );
        }
    }

    #[test]
    fn ids_are_unique() {
        let mut seen: Vec<&str> = Vec::new();
        for a in ACTIONS {
            assert!(!seen.contains(&a.id), "duplicate action id {:?}", a.id);
            seen.push(a.id);
        }
    }

    #[test]
    fn an_unknown_action_is_refused_before_the_platform_is_touched() {
        // Otherwise a binding saved against a newer version of this addon
        // reaches the media session as a typo and fails with whatever the
        // platform says, rather than with what is actually wrong.
        let mut addon = MediaSession::new();
        match addon.perform("teleport", &Invocation::bare()) {
            Err(AddonError::NoSuchAction(a)) => assert_eq!(a, "teleport"),
            other => panic!("expected NoSuchAction, got {other:?}"),
        }
    }

    #[test]
    fn every_action_takes_an_optional_target() {
        // Optional is the point. "Whatever is playing" is what most people want
        // from a play/pause key, and making the target required would break
        // every binding written before it existed.
        for a in ACTIONS {
            let target = a.param("app").expect("every action can be aimed");
            assert!(!target.required);
            assert_eq!(target.kind, ParamKind::App);
            assert_eq!(a.trigger, Trigger::Momentary);
        }
    }
}
