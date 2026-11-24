//! links the program's icon into the exe, and copies the ffmpeg dlls beside the built program and its tests, so
//! `cargo run` and `cargo test` start without a path set.

use std::path::PathBuf;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_WINDOWS").is_err() {
        return;
    }
    // the icon as scripts\icon.ps1 compiled it: the linker takes a resource file as it is. the program only, not
    // its tests
    let icon = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("icon").join("eyepiece.res");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rustc-link-arg-bins={}", icon.display());
    let Ok(ffmpeg) = std::env::var("FFMPEG_DIR") else { return };
    // OUT_DIR is target/<profile>/build/<pkg>-<hash>/out: the program lands three levels up, its tests in deps beside it
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let Some(profile) = out.ancestors().nth(3) else { return };
    let Ok(dlls) = std::fs::read_dir(PathBuf::from(ffmpeg).join("bin")) else { return };
    for dll in dlls.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "dll")) {
        for dir in [profile.to_path_buf(), profile.join("deps")] {
            let to = dir.join(dll.file_name().unwrap());
            if !to.exists() {
                let _ = std::fs::copy(&dll, &to);
            }
        }
    }
}
