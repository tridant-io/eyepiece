# eyepiece

a review player for vfx and install content, by tridant. it opens a clip at once, steps one true frame at a time, shows true pixels, and says exactly what the file is.

**status: phase 1, first slice, windows.** it plays hap from the command line or a file dropped on the window. no controls yet: the title bar carries the readout. prores, h.264, hevc, the control row, zoom and the metadata panel are the rest of phase 1. macos comes after.

what has been played on real files: hap, hap alpha, hap q, hap r and hap hdr at 4K and 60 frames a second, and hap q at 8K. hap q alpha and alpha-only follow the published spec, but no file of either was at hand. hap hdr goes to the screen as stored: values above 1.0 clip, with no tone map and no chip saying so yet. dropping a file on the window is written and not yet tested.

## what is here

| path | what |
|---|---|
| `src/` | the player. `media/` reads files (ffmpeg for containers, our own code for hap), `play.rs` paces frames to the display, `win/` is the window and the direct3d renderer. |
| `tests/media/` | tiny clips the unit tests read, and the script that made them |
| `scripts/` | setup, and the build that packages and signs |
| `docs/design/plan.html` | the approved design, rev c (2026-11-23). open it in a browser. |
| `docs/phase-0.md` | the gates the design had to pass before any player code: what each proves and what it found |
| `docs/adr/` | decisions that are hard to reverse, one file each |
| `GLOSSARY.md` | the project's words |
| `spike/` | the throwaway gate programs. not the player, and the player does not use them. |

## run it (windows)

needs rust (stable, msvc target) and the visual studio c++ build tools.

```powershell
# once: fetches the pinned lgpl ffmpeg and a libclang into third_party (ignored by git)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup.ps1
cargo test
cargo run --release -- some-clip.mov
# end to end: drives the built program with real keys and checks what it says (a window opens for a few seconds)
powershell -NoProfile -ExecutionPolicy Bypass -File tests\e2e.ps1
# pacing on real clips of your own: plays each for 10 s and counts refreshes that went by without the frame due
powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\play.ps1 -Clips a.mov,b.mov"
```

keys so far: space plays and pauses. F and G, or the left and right arrows, step one frame; shift with an arrow steps ten. shift+F and shift+G, or home and end, go to the first and last frame. the wheel scrubs, one frame a notch. drop a file on the window to open it.

while a clip plays the title bar is left alone; standing still it reads `name · frame / count · size · codec · rate`, and ` · n late` if any refresh went by without the frame that was due.

## deploy

```powershell
# tests, release build, dist\eyepiece with the four ffmpeg libraries and their license, signed as "Tridant, Inc."
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1
# the same, then copied to %LOCALAPPDATA%\Programs\eyepiece
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -Install
```

signing uses azure artifact signing and needs `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` in the user's environment, the windows sdk's signtool, and microsoft's artifact signing client tools; `-NoSign` skips it. there is no installer yet: the folder is the install. macos builds will be signed on tec-mba with the owner's personal developer id for now.

## the gates (phase 0)

the gate programs live in `spike/`, with their own setup and their own test clips, and are kept until the mac renderer has rerun them.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File spike\setup.ps1
# makes the test clips in spike\media (about 10 GB with -Big; needs a full ffmpeg on the path for its encoders)
powershell -NoProfile -ExecutionPolicy Bypass -File spike\media.ps1 -Big
powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate.ps1 gate1_zerocopy spike\media\hevc10_4k_5994.mp4
# gate 6 is its own script: a scratch samba share in a small wsl distro (eyepiece-gate6), read from here or from a mac
powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate6.ps1
```

on windows: `gate1_zerocopy`, `gate2_present`, `gate3_ramp`, `gate4_coldstart`, `gate5_share`, `gate6_browse`, and the measurements `hap_demux`, `hap_speed`, `hap_play`, `decode_speed`, `devtime`. a gate ends with `PASS` or `FAIL`; a measurement only prints numbers.

on macos (rust and apple's command line tools; the mac has no github access, so `spike/` is copied there with tar over ssh):

```sh
# once: builds an lgpl shared ffmpeg 9.0 with videotoolbox into spike/third_party
sh spike/mac/setup.sh
sh spike/mac/gate.sh gate5_share some-small-clip.mov
# anything that opens a window or needs the keychain runs inside the login session
sh spike/mac/session-run.sh gate2 spike/mac/target/release/gate2_present some-10-bit-hevc.mp4 20
```

there: `gate1_zerocopy`, `gate3_ramp`, `metal_caps`, `gate5_share`, `hap_lane`, `decode_speed` and `vt_check` from any shell; `gate2_present`, `gate4_coldstart`, `hap_play` and `sign-test.sh` (signing and notarization) through `session-run.sh`. what each one found is in `docs/phase-0.md`.
