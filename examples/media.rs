//! What the media addon can see and do, against whatever is playing.
//!
//!     cargo run -p nobble-addon-media --example media          # report only
//!     cargo run -p nobble-addon-media --example media next     # and skip a track
//!
//! Exists because the interesting half of this addon cannot be unit tested:
//! there is no media session in CI, and a mock of one would only prove the
//! mock works.

use nobble_addon_media::MediaSession;
use nobble_addon_sdk::{Addon, Availability, Invocation};

fn main() {
    let mut addon = MediaSession::new();
    println!("{} — {}", addon.name(), addon.description());
    match addon.availability() {
        Availability::Ready => println!("available: yes"),
        Availability::Unavailable(why) => println!("available: no — {why}"),
    }
    match addon.now_playing() {
        Some(n) => println!(
            "playing : {} — {} ({}) from {}",
            if n.title.is_empty() {
                "<no title>"
            } else {
                &n.title
            },
            if n.artist.is_empty() {
                "<no artist>"
            } else {
                &n.artist
            },
            if n.playing { "playing" } else { "paused" },
            n.source
        ),
        None => println!("playing : nothing"),
    }
    println!(
        "actions : {}",
        addon
            .actions()
            .iter()
            .map(|a| a.id)
            .collect::<Vec<_>>()
            .join(", ")
    );

    if let Some(action) = std::env::args().nth(1) {
        println!("--- performing {action} ---");
        match addon.perform(&action, &Invocation::bare()) {
            Ok(()) => println!("ok"),
            Err(e) => println!("failed: {e}"),
        }
    }
}
