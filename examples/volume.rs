//! What the volume addon can see and do.
//!
//!     cargo run -p nobble-addon-volume --example volume              # report only
//!     cargo run -p nobble-addon-volume --example volume 8000         # set the system volume
//!     cargo run -p nobble-addon-volume --example volume 8000 spotify # ...or one application's
//!     cargo run -p nobble-addon-volume --example volume sweep        # as a fader would
//!
//! The position is given in the units a fader actually produces — 14-bit,
//! `0..=16383` — rather than as a percentage, because that is what reaches the
//! addon and the conversion is the part worth exercising.
//!
//! Exists because the interesting half of this addon cannot be unit tested:
//! there is no audio endpoint in CI, and a mock of one would only prove the
//! mock works. The taper is pure and *is* unit tested; everything below the
//! `platform` boundary is not.

use std::collections::BTreeMap;
use std::time::Duration;

use nobble_addon_sdk::{Addon, Availability, FADER_MAX, Invocation};
use nobble_addon_volume::{Volume, taper};

fn main() {
    let mut addon = Volume::new();
    println!("{} — {}", addon.name(), addon.description());
    match addon.availability() {
        Availability::Ready => println!("available: yes"),
        Availability::Unavailable(why) => println!("available: no — {why}"),
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    let target = args.get(1).cloned();
    let params: BTreeMap<String, String> = target
        .clone()
        .map(|app| [("app".to_owned(), app)].into_iter().collect())
        .unwrap_or_default();

    report(&addon, target.as_deref());

    match args.first().map(String::as_str) {
        None => {}
        Some("sweep") => sweep(&mut addon, &params, target.as_deref()),
        Some(position) => {
            let Ok(value) = position.parse::<u16>() else {
                println!("position should be 0..={FADER_MAX}, or the word \"sweep\"");
                return;
            };
            set(&mut addon, &params, value, target.as_deref());
        }
    }
}

/// Walk the fader from bottom to top, as a hand would.
///
/// Every step is dispatched, which is deliberately *not* what the daemon does:
/// there the coalescer collapses a sweep to wherever the fader ended up. This
/// shows what the curve sounds like, which is the thing that cannot be checked
/// any other way.
fn sweep(addon: &mut Volume, params: &BTreeMap<String, String>, target: Option<&str>) {
    println!("--- sweeping, 0 to full over about five seconds ---");
    for step in 0..=20u16 {
        let value = step * (FADER_MAX / 20);
        set(addon, params, value, target);
        std::thread::sleep(Duration::from_millis(250));
    }
}

fn set(addon: &mut Volume, params: &BTreeMap<String, String>, value: u16, target: Option<&str>) {
    let invocation = Invocation::moved(params, value);
    let scalar = invocation.fraction().map_or(0.0, taper);
    match addon.perform("set", &invocation) {
        Ok(()) => println!("  {value:>5} -> scalar {scalar:.4}  ({}%)", pct(scalar)),
        Err(e) => println!("  {value:>5} -> failed: {e}"),
    }
    let _ = target;
}

fn report(addon: &Volume, target: Option<&str>) {
    match addon.level(target) {
        Some(level) => println!(
            "level    : {:.4} ({}%) for {}",
            level,
            pct(level),
            target.unwrap_or("the system")
        ),
        None => println!("level    : unknown for {}", target.unwrap_or("the system")),
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn pct(scalar: f32) -> u32 {
    (scalar * 100.0).round() as u32
}
