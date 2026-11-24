//! screenshot files: png, in one local folder, never beside the media and never over an earlier screenshot.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

/// `pixels` is red, green, blue, alpha, 16 bits each (65535 is 1.0), row by row. writes a png without alpha:
/// 16 bits a channel when `deep`, else 8 (rounded to nearest). never overwrites: `stem.png`, then `stem (2).png`, and so on.
/// creates the folder if it is missing. returns the file written.
pub fn save(pixels: &[u16], width: u32, height: u32, deep: bool, folder: &Path, stem: &str) -> Result<PathBuf, String> {
    if pixels.len() != width as usize * height as usize * 4 {
        return Err(format!("{} values for a {width} × {height} picture", pixels.len()));
    }
    let rgb = pixels.as_chunks::<4>().0.iter().flat_map(|p| &p[..3]);
    let data: Vec<u8> = if deep { rgb.flat_map(|v| v.to_be_bytes()).collect() } else { rgb.map(|&v| ((v as u32 * 255 + 32767) / 65535) as u8).collect() };
    // encoded before any file exists, so a failure leaves none
    let mut png = Vec::new();
    let mut encoder = png::Encoder::new(&mut png, width, height);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(if deep { png::BitDepth::Sixteen } else { png::BitDepth::Eight });
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&data).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut n = 1;
    loop {
        let path = folder.join(if n == 1 { format!("{stem}.png") } else { format!("{stem} ({n}).png") });
        // only ever a new file, so two saves at once cannot take the same name
        match fs::OpenOptions::new().write(true).create_new(true).open(&path) {
            Err(e) if e.kind() == ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(format!("{}: {e}", path.display())),
            Ok(mut file) => {
                let written = file.write_all(&png);
                drop(file);
                return match written {
                    Ok(()) => Ok(path),
                    Err(e) => {
                        // a part of an image is not left looking like a screenshot
                        let _ = fs::remove_file(&path);
                        Err(format!("{}: {e}", path.display()))
                    }
                };
            }
        }
    }
}

/// where screenshots go when the settings name no folder: the system's Pictures folder, in `eyepiece`.
pub fn folder() -> PathBuf {
    // the temporary folder only when the system names no pictures folder: still local, still not beside the media
    pictures().unwrap_or_else(std::env::temp_dir).join("eyepiece")
}

/// the file name for a frame of a clip, without extension: the clip's name without its extension, and the frame number padded as the readout pads it.
/// `wall_v012.mov` at frame 742 is `wall_v012.f000742`, as the design's toast names it.
pub fn stem(clip_name: &str, frame: usize) -> String {
    let name = Path::new(clip_name).file_stem().map_or(clip_name.into(), |s| s.to_string_lossy());
    format!("{name}.f{frame:06}")
}

#[cfg(windows)]
fn pictures() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::System::Com::CoTaskMemFree;
    use windows::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath, KF_FLAG_DEFAULT};
    unsafe {
        let found = SHGetKnownFolderPath(&FOLDERID_Pictures, KF_FLAG_DEFAULT, None).ok()?;
        let path = std::ffi::OsString::from_wide(found.as_wide());
        CoTaskMemFree(Some(found.0 as *const _));
        Some(path.into())
    }
}

// macos. linux comes in phase 5
#[cfg(not(windows))]
fn pictures() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("HOME")?).join("Pictures"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a folder of its own for one test, which `save` makes. it removes the images named to it one at a time, then
    /// the folder
    struct Scratch(PathBuf, &'static [&'static str]);

    impl Scratch {
        fn new(test: &str, images: &'static [&'static str]) -> Scratch {
            Scratch(std::env::temp_dir().join(format!("eyepiece-shot-{test}-{}", std::process::id())), images)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            for name in self.1 {
                let _ = fs::remove_file(self.0.join(name));
            }
            let _ = fs::remove_dir(&self.0);
        }
    }

    // two pixels. 128 is just under half an 8-bit step and 129 just over; the alphas are not written
    const PIXELS: [u16; 8] = [0, 128, 129, 65535, 65535, 25828, 1000, 0];

    fn read_back(path: &Path) -> (png::ColorType, png::BitDepth, Vec<u8>) {
        let mut reader = png::Decoder::new(std::io::BufReader::new(fs::File::open(path).unwrap())).read_info().unwrap();
        let mut buf = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut buf).unwrap();
        buf.truncate(info.buffer_size());
        (info.color_type, info.bit_depth, buf)
    }

    #[test]
    fn both_depths_read_back() {
        let scratch = Scratch::new("depths", &["deep.png", "plain.png"]);
        assert!(save(&PIXELS[..4], 2, 1, true, &scratch.0, "short").is_err());
        let deep = save(&PIXELS, 2, 1, true, &scratch.0, "deep").unwrap();
        let rgb = [0u16, 128, 129, 65535, 25828, 1000];
        assert_eq!(read_back(&deep), (png::ColorType::Rgb, png::BitDepth::Sixteen, rgb.iter().flat_map(|v| v.to_be_bytes()).collect()));
        let plain = save(&PIXELS, 2, 1, false, &scratch.0, "plain").unwrap();
        // 25828 is 100.498 steps of 257, and 1000 is 3.89
        assert_eq!(read_back(&plain), (png::ColorType::Rgb, png::BitDepth::Eight, vec![0, 0, 1, 255, 100, 4]));
    }

    #[test]
    fn never_overwrites() {
        const NAMES: [&str; 3] = ["clip.f000007.png", "clip.f000007 (2).png", "clip.f000007 (3).png"];
        let scratch = Scratch::new("names", &NAMES);
        let written: Vec<PathBuf> = (0..3).map(|_| save(&PIXELS, 2, 1, false, &scratch.0, "clip.f000007").unwrap()).collect();
        assert_eq!(written, NAMES.map(|n| scratch.0.join(n)));
    }

    #[test]
    fn stem_is_the_name_and_the_padded_frame() {
        assert_eq!(stem("wall_main_v012.mov", 742), "wall_main_v012.f000742");
        assert_eq!(stem("shot.v2.mov", 0), "shot.v2.f000000");
        assert_eq!(stem("no extension", 1234567), "no extension.f1234567");
    }
}
