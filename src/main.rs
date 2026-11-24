//! eyepiece: a review player for vfx and install content.
//! usage: eyepiece [file]

// no console window behind the player in a release build
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod event;
mod media;
mod play;
mod ui;
#[cfg(windows)]
mod win;

fn main() {
    let first = std::env::args().nth(1);
    #[cfg(windows)]
    if let Err(why) = win::run(first) {
        eprintln!("eyepiece: {why}");
        std::process::exit(1);
    }
    #[cfg(not(windows))]
    {
        let _ = first;
        eprintln!("eyepiece: this build runs on windows only so far");
        std::process::exit(1);
    }
}
