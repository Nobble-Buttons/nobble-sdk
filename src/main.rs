//! The volume addon, as a process.
//!
//! The same three lines as every other addon binary, which is the point: what
//! an author writes is the addon, and the loop belongs to the SDK.

fn main() {
    nobble_addon_sdk::run(nobble_addon_volume::Volume::new());
}
