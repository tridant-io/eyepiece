//! names the build for about (the short commit and the day, as EYEPIECE_COMMIT and EYEPIECE_BUILT), links the
//! program's icon into the exe, and copies the ffmpeg dlls beside the built program and its tests, so `cargo run` and
//! `cargo test` start without a path set.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    let git = |args: &[&str]| Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    // built again whenever the checkout moves to another commit: git logs every move of HEAD
    if let Some(log) = git(&["rev-parse", "--path-format=absolute", "--git-path", "logs/HEAD"]) {
        println!("cargo:rerun-if-changed={log}");
    }
    println!("cargo:rustc-env=EYEPIECE_COMMIT={}", git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into()));
    println!("cargo:rustc-env=EYEPIECE_BUILT={}", today());
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

/// the day, in utc, as YYYY-MM-DD (howard hinnant's days-to-civil)
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 { shifted + 3 } else { shifted - 9 };
    let year = year_of_era + era * 400 + (month <= 2) as i64;
    format!("{year:04}-{month:02}-{day:02}")
}
