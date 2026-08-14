//! Does a long-lived addon keep following the current media session?
//!
//!     cargo run -p nobble-addon-media --example follow            # 5 minutes, every 2 s
//!     cargo run -p nobble-addon-media --example follow 10 1       # 10 minutes, every 1 s
//!
//! The question this answers is the one the old comment on `session()` raised
//! and SC-005 forced: the session manager is now kept rather than requested on
//! every call, and a kept object can go stale. The failure it feared is silent
//! — a command reaching an application that has closed — so it cannot be found
//! by reading a return value. It has to be watched across a real application
//! change.
//!
//! Unlike `examples/media`, this holds **one addon for the whole run**, on one
//! thread, which is what makes the cache observable at all: a fresh process has
//! a fresh manager and would pass no matter what.
//!
//! Run it, then change what is playing underneath it — start Spotify, stop it,
//! start a browser tab, close the tab — and read the `source` column. It should
//! name the application that is currently playing at every tick, and go to
//! `nothing` when none is. A `source` that stays on an application you have
//! closed is the staleness the TTL exists to bound.

use std::time::{Duration, Instant};

use nobble_addon_media::MediaSession;
use nobble_addon_sdk::{Addon, Availability, Reading};

fn main() {
    let mut args = std::env::args().skip(1);
    let minutes: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(5);
    let every: u64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(2);

    let mut addon = MediaSession::new();
    println!("watching for {minutes} minutes, every {every}s. Change what is playing underneath it.");
    println!("{:>6}  {:<12}  {:<8}  {:<34}  title", "at", "available", "playing", "source");

    let start = Instant::now();
    let deadline = Duration::from_secs(minutes * 60);
    while start.elapsed() < deadline {
        // Deliberately all three, in the order the daemon calls them: the
        // signal poll reads `read_signals`, the interface reads `availability`,
        // and `perform` resolves the same session both of those did. If the
        // manager goes stale, they go stale together.
        let available = match addon.availability() {
            Availability::Ready => "yes".to_owned(),
            Availability::Unavailable(_) => "no".to_owned(),
        };
        let playing = match addon.read_signals() {
            Reading::Values(v) => v
                .iter()
                .find(|(id, _)| *id == "playing")
                .map_or("?".to_owned(), |(_, b)| b.to_string()),
            Reading::Unavailable(_) => "none".to_owned(),
        };
        let (source, title) = addon.now_playing().map_or_else(
            || ("nothing".to_owned(), String::new()),
            |n| (n.source, n.title),
        );

        println!(
            "{:>5}s  {available:<12}  {playing:<8}  {source:<34}  {title}",
            start.elapsed().as_secs()
        );
        std::thread::sleep(Duration::from_secs(every));
    }
}
