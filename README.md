# eyepiece

a review player for vfx and install content, by tridant. it opens a clip at once, steps one true frame at a time, shows true pixels, and says exactly what the file is.

**status: phase 1, in progress, windows.** it plays hap, notchlc, prores, h.264 and hevc, and stills (png, jpeg, tiff, tga, dpx, exr: one file, or a numbered run of them as one clip), from the command line or a file dropped on the window. the window has no title bar: the controls are drawn by the player to the approved design, and come up when the pointer moves. so far those are the strip along the top (the clip's tab, minimize, maximize, close), the bar along the bottom (previous frame, play, next frame, the frame or timecode readout; then loop, speed, zoom and the view, each with its menu, and fullscreen; the full-width timebar), and the chips that say what is not as the file has it (loop analysis, speed, mirror/flip/rotate and single channels, zoom with the pixel under the pointer). still to come on the bar: the metadata panel, screenshots, volume. then tabs, the browser, image sequences, and macos.

the look follows `docs/design/plan.html`: its colors, measurements, icons and type (ibm plex mono and archivo, in `assets/fonts` with their licenses). checked by putting the window's own pixels beside the design's mock at the same scale. not yet looked at by a person on a real screen: the shadow, resizing from the edges, snapping, and display scales other than 100%.

what has been played on real files, at 4K and 60 frames a second:

- hap, hap alpha, hap q, hap r and hap hdr, and hap q at 8K: our own code, blocks to the gpu as stored. hap q alpha and alpha-only follow the published spec, but no file of either was at hand. hap hdr goes to the screen as stored: values above 1.0 clip, with no tone map and no chip saying so yet.
- h.264 and 10-bit hevc: the system's hardware decoder, drawn from the decoder's own surface with no copy. when the gpu does not take a stream the same clip decodes in software.
- prores 422 hq: ffmpeg in software, one frame at a time on every core. prores 4444 xq at 4K60 does not hold its rate in software on the dev machine (about 30 frames a second), as phase 0 measured.

stills and sequences have only been opened as the small files made for the tests. a numbered run opens as one clip from any of its files, on that file's frame, at the rate the file states (exr can) or 24 assumed, and the title says "assumed". a frame with no file is marked amber on the timebar and says so when shown. 8-bit files stay 8-bit, 10, 12 and 16-bit ones go to the screen as 16-bit, and exr as float through the srgb curve with a chip saying so.

known gaps:

- **notchlc opens and steps, but does not play at speed yet.** ffmpeg decodes one notchlc frame on one core and the player decodes one frame at a time: a 4K60 clip from touchdesigner played at about 10 frames a second on the dev machine (2026-11-23, 165 late refreshes in 3 s). phase 0 measured 100 a second with every core decoding; putting several frames on several cores is being built. values read as 12-bit.
- **4K sequences do not play at speed yet.** one frame is decoded at a time: on the dev machine, busy, a 4K frame took 55 ms (tga) to 426 ms (16-bit png), against the 42 ms a frame has at 24 a second. decoding several frames at once and keeping more of them in memory is not built. exr values above 1.0 stop at white (no tone map), exr compressions other than none, rle and zip are untested, and the rate of a sequence cannot be typed in yet. for exr the pixel readout gives what the screen is sent, on a 16-bit scale, not the file's own numbers.
- **stepping back in h.264 and hevc is instant only inside the frames kept.** the player keeps the frames it last decoded, as many as fit in 512 MB (about 20 at 4K, 32 at most), and a step back inside them is one or two refreshes. the step that leaves them decodes again from the key frame before: on the dev machine that one step took 357 ms on 4K hevc with a key frame every 60, and up to 1.5 s on 4K h.264 with a key frame every 250. `tests\backstep.ps1` measures it on a clip of your own. hap and prores frames stand alone and are not affected.
- **a cold start is slower than the 250 ms target.** from the process starting to the first picture, on the dev machine, six launches each: hap q 285 ms, prores 332 ms, h.264 360 ms, hevc 376 ms (4K clips, medians; `tests\launch.ps1`). making the graphics device is most of it, as phase 0 found.
- the installed folder is 113 MB against a 60 MB target: ffmpeg's decoder library is 100 MB of it, built with every decoder there is. a build with only ours is later work.
- chroma is read at the nearest sample, not blended at the file's siting yet.
- a file that does not name its color matrix gets the usual assumption (bt.709 at 720 lines and up, bt.601 below), and nothing on screen says so until the metadata panel exists.
- variable frame rates do not play. mkv and fragmented mp4 do not play: they carry no table of frames, and counting them by reading the file is not built.
- the frame count leaves out what an edit list hides (a clip cut from a longer one without re-encoding), which is tested. pictures that are stored but cannot be decoded are not yet left out, and the counts have only been checked on clips made for the tests, not on files from cameras or other tools.
- prores decodes in software only, with ffmpeg's decoder; alpha in a prores file is not drawn.
- dropping a file on the window is written and not yet tested.
- playing backward (J) in h.264 and hevc runs in bursts: frames are decoded forward from a key frame and only the last ones are kept, so every 20 frames or so at 4K it stops to decode again. hap and prores play backward evenly.
- loop analysis does not yet say how much of the picture changes across the seam, and its pause on black is a fixed 320 ms.
- a folder's "loop all" comes with playing folders. the settings file has a place for it.
- the pixel values in the zoom chip are the red, green and blue the screen is sent, on the scale of the file's bit depth (0 to 255 for 8-bit, 0 to 1023 for 10-bit). they are read from the frame at its own size, not from the window. they are not the stored y'cbcr codes, and they stop at the top of the scale: a hap hdr value above 1.0 reads as the top.

the title bar also says what went wrong: ` · n late` (refreshes that went by without the frame that was due), ` · n could not be shown` (frames the file could not give), and ` · reading` while the frame it names is still on its way.

## what is here

| path | what |
|---|---|
| `src/` | the player. `media/` reads files (ffmpeg for containers, our own code for hap), `play.rs` paces frames to the display (speed, shuttle, looping, loop analysis), `view.rs` places and turns the picture, `ui/` draws the controls, `settings.rs` is what is kept between runs, `win/` is the window and the direct3d renderer. built and tested but not wired in yet: `browse/` (folders, from their listing alone), `media/meta.rs` (the metadata panel's facts), `media/watch.rs` (noticing a file changed), `shot.rs` (screenshot files). |
| `tests/media/` | tiny clips the unit tests read, and the script that made them |
| `scripts/` | setup, and the build that packages and signs |
| `docs/design/plan.html` | the approved design, rev c (2026-11-23). open it in a browser. |
| `docs/phase-0.md` | the gates the design had to pass before any player code: what each proves and what it found |
| `docs/adr/` | decisions that are hard to reverse, one file each |
| `GLOSSARY.md` | the project's words |
| `spike/` | the throwaway gate programs. not the player, and the player does not use them. |

## run it (windows)

needs rust (stable, msvc target) and the visual studio c++ build tools. the tests need a gpu with a video decoder: one unit test and the end-to-end test check hardware decoding, and fail on a machine without it.

```powershell
# once: fetches the pinned lgpl ffmpeg and a libclang into third_party (ignored by git)
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup.ps1
cargo test
cargo run --release -- some-clip.mov
# end to end: drives the built program with real keys and checks what it says (a window opens for a few seconds)
powershell -NoProfile -ExecutionPolicy Bypass -File tests\e2e.ps1
# pacing on real clips of your own: plays each for 10 s; fails a clip that did not open, lost frames, or ran late
powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\play.ps1 -Clips a.mov,b.mov"
# measurements on clips of your own: start to first picture, and what stepping back costs
powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\launch.ps1 -Clips a.mov,b.mp4"
powershell -NoProfile -ExecutionPolicy Bypass -File tests\backstep.ps1 -Clip a.mp4 -Frames 600 -KeyEvery 60
```

keys so far: space plays and pauses. J plays backward, L forward, each press doubling up to 8×, and K stops. - and = step the speed (0.1× to 8×), backspace is 1× again. R turns looping off and on; A is loop analysis: the clip's last seconds running into its first, over and over, with a pause on black for the trip back. F and G, or the left and right arrows, step one frame; shift with an arrow steps ten. shift+F and shift+G, or home and end, go to the first and last frame. the wheel scrubs, one frame a notch. 0 fits the picture, 1 is 1:1 and 2 is 2×, about the pointer when it is over the picture; shift with the wheel zooms one stop a notch about the pointer (12.5% to 6400%, with fit as a stop of its own); alt with the arrows moves a zoomed picture an eighth of the window. D shows a grid of tiles and C the center lines and circle; H mirrors, V flips, Q turns the picture a quarter (the view changes, never the file, and a chip says so). P is fullscreen, shift+P fullscreen on the next display. T switches the readout between frames and timecode. esc leaves fullscreen first, then closes a menu. drop a file on the window to open it.

with the pointer: move it and the controls come up; they go 2.4 s after it rests off them, or when it leaves the window. a plain click on the picture shows or hides them. a drag on the strip moves the window, and so does a drag on the picture while all of it is in the window; when it is larger than the window, a drag moves the picture, as far as its edge. a double click on the picture goes between fit and the last zoom and place (1:1 the first time); a double click on the strip maximizes the window. the number at the right of the bar is the zoom: a click opens its menu (fit, 1:1, 2×, 4×, 8×, a slider, a field to type a percentage into). while a zoom is set, a chip at the top right gives it, says whether every video pixel is a whole number of display pixels (exact) or not (scaled), and names the pixel under the pointer with its values; it goes 2 s after the pointer rests. from 1000% up, a dark line marks every pixel's edge. the bar's other menus: loop (off, one, and the loop analysis switch with its seconds), speed (five stops, a slider, a field), and the view (grid with its tile size, center lines and circle, one channel alone, mirror, flip, rotate). a click or a drag on the timebar goes to that frame, and a click on the readout switches between frames and timecode (counted from 00:00:00:00; the file's own timecode is not read yet).

the window's title, which the taskbar shows and the tests read, is `name · frame / count · size · codec, how it is decoded · rate`, then whatever is not as a clip opens (looping off, loop analysis, speed, the look, the guides, the zoom and the pixel under the pointer); it is left alone while a clip plays.

what is kept between runs is in `%APPDATA%\eyepiece\settings.txt`, one `key = value` a line, editable by hand; a line taken out gets its default. so far the player keeps the readout (frames or timecode), looping, the loop analysis seconds, and the guides with their tile size. speed, zoom and the look always start as the file has them.

the tests drive the controls with posted messages, not the real pointer and keyboard. `EYEPIECE_POSTED_POINTER=1` tells the player so: it then listens only to pointer messages carrying the tests' mark (0x8000 in the first parameter) and to keys with no scan code, and takes what is held with a posted key from the message's own low bits (1 shift, 2 alt, 4 control). a person using the machine during a test run does not disturb it. `tests\e2e.ps1` also points `APPDATA` at a folder of its own, so it starts from the defaults and leaves your settings alone.

the timing checks (`play.ps1`, `launch.ps1`, `backstep.ps1`) mean nothing on a busy machine: on 2026-11-23, with other test suites holding this pc at 97% cpu, a clip that had run with 2 late refreshes in 10 s showed 246 in 6 s, with the same build.

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
