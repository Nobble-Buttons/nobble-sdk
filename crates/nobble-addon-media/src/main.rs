//! The media addon, as a process.
//!
//! Everything an addon binary needs to be: construct the addon, hand it to the
//! SDK, and let the SDK own the loop. If this were longer it would be a sign
//! that the SDK was not carrying enough, because a third-party addon has to be
//! able to write exactly this much and no more.

fn main() {
    nobble_addon_sdk::run(nobble_addon_media::MediaSession::new());
}
