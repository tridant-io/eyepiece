//! eyepiece: a video review player for video professionals. extremely fast, lightweight, and only what you need.
//! usage: eyepiece [file]

// no console window behind the player in a release build
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod browse;
mod audio;
mod event;
mod folder;
mod markup;
mod media;
mod play;
mod pictures;
mod player;
mod preview;
mod settings;
mod shot;
mod timecode;
mod ui;
mod update;
mod view;
#[cfg(target_os = "macos")]
mod mac;
#[cfg(windows)]
mod win;
// the system's half of the player: its window, its device and its painters
#[cfg(target_os = "macos")]
use mac as sys;
#[cfg(windows)]
use win as sys;

fn main() {
    let first = std::env::args().nth(1);
    if let Err(why) = sys::run(first) {
        eprintln!("eyepiece: {why}");
        std::process::exit(1);
    }
}
