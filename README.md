# eyepiece

a review player for vfx and install content, by tridant. it opens a clip at once, steps one true frame at a time, shows true pixels, and says exactly what the file is.

**status: phase 1, in progress, windows.** it plays hap, notchlc, prores, h.264 and hevc, and stills (png, jpeg, tiff, tga, dpx, exr: one file, or a numbered run of them as one clip), from the command line or a file dropped on the window. the window has no title bar: the controls are drawn by the player to the approved design, and come up when the pointer moves. so far those are the strip along the top (the tabs, the + for a new one, minimize, maximize, close), the bar along the bottom (previous frame, play, next frame, the frame or timecode readout; then loop, speed, zoom and the view, each with its menu, and fullscreen; the full-width timebar), and the chips that say what is not as the file has it (loop analysis, speed, mirror/flip/rotate and single channels, zoom with the pixel under the pointer). started with no file, or in a new tab, it shows the browser, which so far is one plain list of the folder you are in; the design's columns, grid and metadata column are being built. the bar also has the metadata panel and screenshots; volume is still to come. then macos.

the look follows `docs/design/plan.html`: its colors, measurements, icons and type (ibm plex mono and archivo, in `assets/fonts` with their licenses), with two corrections the owner asked for on seeing it: the bar's row of controls sits in the middle of the bar (between its top line and the timebar), and every line of text is centered by its figures and capitals, not by its line's box. checked by putting the window's own pixels beside the design's mock at the same scale. not yet looked at by a person on a real screen: the shadow, resizing from the edges, snapping, and display scales other than 100%.

what has been played on real files, at 4K and 60 frames a second:

- hap, hap alpha, hap q, hap r and hap hdr, and hap q at 8K: our own code, blocks to the gpu as stored. hap q alpha and alpha-only follow the published spec, but no file of either was at hand. hap hdr goes to the screen as stored: values above 1.0 clip, with no tone map and no chip saying so yet.
- h.264 and 10-bit hevc: the system's hardware decoder, drawn from the decoder's own surface with no copy. when the gpu does not take a stream the same clip decodes in software.
- prores 422 hq: ffmpeg in software, one frame at a time on every core. prores 4444 xq at 4K60 does not hold its rate in software on the dev machine (about 30 frames a second), as phase 0 measured.

stills and sequences have only been opened as the small files made for the tests. a numbered run opens as one clip from any of its files, on that file's frame, at the rate the file states (exr can) or 24 assumed, and the title says "assumed". a frame with no file is marked amber on the timebar and says so when shown. 8-bit files stay 8-bit, 10, 12 and 16-bit ones go to the screen as 16-bit, and exr as float through the srgb curve with a chip saying so.

known gaps:

- **screenshots carry no alpha yet**, and no caption, guides or markup. the screenshot of the view is 8 or 16 bits by the file's depth; the copy on the clipboard is always 8.
- the metadata panel reads timecode through ffmpeg (the design wants our own read of the track), says "not read for this codec yet" for alpha in h.264 and hevc, and for stills gives no color facts yet.
- **a dragged window does not snap to the screen's edges.** the move is the player's own, because the system's move, started from the picture, sometimes did not start. win + arrows, the maximize button and a double click on the strip still arrange the window.
- **tabs keep their clips open.** every tab holds its decoder and the frames it keeps (up to 512 MB each for h.264 and hevc), so many 4K tabs use a lot of memory; nothing is let go until a tab is closed. more tabs than fit the strip narrow to 64 wide and then run under the window's buttons: the strip does not scroll yet.
- **notchlc plays at speed, but a lone frame takes one core's whole decode.** ffmpeg decodes a notchlc frame on one core, so the player decodes up to eight at once, each worker with its own decoder, and keeps what it decoded in up to a quarter of ram. a 4K60 clip from touchdesigner, as a benchmark of the reader alone on the dev machine while busy: 119 of 120 frames on time in the first pass, all in the second (medians of five, `plays_at_speed` in src/media/clip.rs); a frame not ready when stepped to took 113 ms. in the player itself (2026-11-23): the second pass ran with no late refresh; the first lost about a second at the start, before frames were read ahead while standing still. that read-ahead is now built and not yet measured: the gpu was busy with other work. values read as 12-bit.
- **4K sequences play at speed in the reader's benchmark, and from ram after the first pass while they fit in a quarter of it; in the window they are not measured yet.** frames decode on up to eight workers at once and stay in memory up to a quarter of ram, least recently wanted out first. 48-frame 4K sequences at 24 a second, dev machine busy, files read from the os cache: exr half zip, 10-bit dpx, 8-bit png and tga had 47 of 48 frames on time in the first pass and 48 in the second (medians of five, `plays_at_speed`). a frame not ready when stepped to takes one whole decode: 64 ms (tga) to 229 ms (png). a sequence longer than a quarter of ram decodes again every pass. reading from a disk or a nas, 16-bit png and the upload of 133 MB float frames to the gpu are not measured. exr values above 1.0 stop at white (no tone map), exr compressions other than none, rle and zip are untested, and the rate of a sequence cannot be typed in yet. for exr the pixel readout gives what the screen is sent, on a 16-bit scale, not the file's own numbers.
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
| `src/` | the player. `media/` reads files (ffmpeg for containers, our own code for hap), `play.rs` paces frames to the display (speed, shuttle, looping, loop analysis), `view.rs` places and turns the picture, `ui/` draws the controls, `settings.rs` is what is kept between runs, `win/` is the window and the direct3d renderer. `browse/` lists folders from their listing alone, `media/meta.rs` reads the metadata panel's facts, `shot.rs` writes screenshot files, `audio/` and `media/watch.rs` (noticing a file changed) are built and tested but not wired in yet. |
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
# dragging the window by its picture, with the real mouse (it borrows the pointer for a few seconds: hands off)
powershell -NoProfile -ExecutionPolicy Bypass -File tests\drag.ps1
# pacing on real clips of your own: plays each for 10 s; fails a clip that did not open, lost frames, or ran late
powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\play.ps1 -Clips a.mov,b.mov"
# measurements on clips of your own: start to first picture, and what stepping back costs
powershell -NoProfile -ExecutionPolicy Bypass -Command "& tests\launch.ps1 -Clips a.mov,b.mp4"
powershell -NoProfile -ExecutionPolicy Bypass -File tests\backstep.ps1 -Clip a.mp4 -Frames 600 -KeyEvery 60
```

one player runs at a time: starting eyepiece again with a file opens that file in a tab of its own in the window already open, which comes to the front (restored if minimized), and the new process ends without a window of its own; with no file it only brings the window forward. if the open player does not answer within 2 s, the new one starts on its own.

keys so far: N opens a new tab and O lays the browser over the one that is up; [ and ] go round the tabs, control + W closes the one that is up and control + shift + T brings the last one closed back, on its frame, zoom and pan. in the browser the arrows move, enter opens, backspace goes up a folder, a letter selects by name and esc goes back to the clip. space plays and pauses. J plays backward, L forward, each press doubling up to 8×, and K stops. shift with - and = steps the speed (0.1× to 8×), backspace is 1× again. R turns looping off and on; A is loop analysis: the clip's last seconds running into its first, over and over, with a pause on black for the trip back. F and G, or the left and right arrows, step one frame; shift with an arrow steps ten. shift+F and shift+G, or home and end, go to the first and last frame. the wheel scrubs, one frame a notch. 0 fits the picture, 1 is 1:1 and 2 is 2×, and - and = zoom out and in one stop, all about the pointer when it is over the picture; shift with the wheel zooms one stop a notch about the pointer (12.5% to 6400%, with fit as a stop of its own); alt with the arrows moves a zoomed picture an eighth of the window. D shows a grid of tiles and C the center lines and circle; H mirrors, V flips, Q turns the picture a quarter (the view changes, never the file, and a chip says so). P is fullscreen, shift+P fullscreen on the next display. S saves a screenshot of the file's own frame at full size (a png, 16 bits a channel when the file is deeper than 8), shift + S saves the view as shown, and control + C copies the frame; I pins the metadata panel. T switches the readout between frames and timecode. esc leaves fullscreen first, then closes a menu. drop a file on the window to open it.

with the pointer: move it and the controls come up; they go 2.4 s after it rests off them, or when it leaves the window. a plain click on the picture shows or hides them. a click on a tab puts it up, the mark on it closes it, and the + opens a new one. a file dropped on the window, or handed over by starting the player again, opens in a tab of its own and never replaces what you are looking at; a file chosen in the browser opens in the tab the browser is in. two tabs holding clips of one size and length change places on the same frame, zoom and pan, so [ and ] flip between two versions. the window grows when a bigger clip is opened and never shrinks. a file that does not open gets a tab that says why. a drag on the strip moves the window, and so does a drag anywhere on the picture while all of it is in the window (the window moves with the very point that was pressed, and a maximized window is put back to its own size first); when it is larger than the window, a drag moves the picture, as far as its edge. a double click on the picture fills the display with it, and another puts the window back (the design had it go between fit and the last zoom; the owner changed it on using it); a double click on the strip maximizes the window. the number at the right of the bar is the zoom: a click opens its menu (fit, 1:1, 2×, 4×, 8×, a slider, a field to type a percentage into). while a zoom is set, a chip at the top right gives it, says whether every video pixel is a whole number of display pixels (exact) or not (scaled), and names the pixel under the pointer with its values; it goes 2 s after the pointer rests. from 1000% up, a dark line marks every pixel's edge. the bar's other menus: loop (off, one, and the loop analysis switch with its seconds), speed (five stops, a slider, a field), and the view (grid with its tile size, center lines and circle, one channel alone, mirror, flip, rotate). a click or a drag on the timebar goes to that frame, and a click on the readout switches between frames and timecode (counted from 00:00:00:00; the file's own timecode is not read yet).

the window's title, which the taskbar shows and the tests read, is `name · frame / count · size · codec, how it is decoded · rate`, then whatever is not as a clip opens (looping off, loop analysis, speed, the look, the guides, the zoom and the pixel under the pointer, the browser over it); it is left alone while a clip plays. screenshots go to `Pictures\eyepiece` (or the settings' `screenshot_folder`), named `clip.f000742.png`, then `clip.f000742 (2).png`: never beside the media and never over an earlier one; a line in the window says what was written. the metadata panel comes up while the pointer rests on its button, a click pins it at the top right, "copy all" puts it on the clipboard, and an amber dot on the button means the file has a fault. every value in it is read from the file, or is dim and says what the file does not state and what is assumed instead. a tab with no clip is titled `eyepiece`, and one whose file did not open `eyepiece · why`.

what is kept between runs is in `%APPDATA%\eyepiece\settings.txt`, one `key = value` a line, editable by hand; a line taken out gets its default. so far the player keeps the readout (frames or timecode), looping, the loop analysis seconds, the guides with their tile size, and the folder the browser was in. speed, zoom and the look always start as the file has them.

the tests drive the controls with posted messages, not the real pointer and keyboard. `EYEPIECE_POSTED_POINTER=1` tells the player so: it then listens only to pointer messages carrying the tests' mark (0x8000 in the first parameter) and to keys with no scan code, and takes what is held with a posted key from the message's own low bits (1 shift, 2 alt, 4 control). a person using the machine during a test run does not disturb it. `tests\e2e.ps1` also points `APPDATA` at a folder of its own, so it starts from the defaults and leaves your settings alone. and it sets `EYEPIECE_INSTANCE` to a name of its own: players started with the same name hand files only to each other, so the tests can check the hand-over without ever reaching a player of yours.

standing still, the player reads ahead what play would show next (one frame for a lane with one worker's worth, up to 32 for stills and notchlc), so play starts on frames that are already there.

the timing checks (`play.ps1`, `launch.ps1`, `backstep.ps1`) mean nothing on a busy machine, and a busy gpu counts (on 2026-11-23, with an encoder, touchdesigner and obs on the gpu, a hap clip that had run with 2 late refreshes in 4 s showed 53): on 2026-11-23, with other test suites holding this pc at 97% cpu, a clip that had run with 2 late refreshes in 10 s showed 246 in 6 s, with the same build.

## deploy

```powershell
# tests, release build, dist\eyepiece with the four ffmpeg libraries, their license and the notices, signed as "Tridant, Inc."
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1
# the same, then copied to %LOCALAPPDATA%\Programs\eyepiece
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\package.ps1 -Install
```

signing uses azure artifact signing and needs `AZURE_TENANT_ID`, `AZURE_CLIENT_ID` and `AZURE_CLIENT_SECRET` in the user's environment, the windows sdk's signtool, and microsoft's artifact signing client tools; `-NoSign` skips it. there is no installer yet: the folder is the install. macos builds will be signed on tec-mba with the owner's personal developer id for now.

`THIRD-PARTY-NOTICES.txt` in the folder is what the build owes the code of others in it: every crate compiled into the program with its license text, ffmpeg (its version, its source, and how to put other libraries in place of ours) and the two typefaces. `scripts\notices.ps1` writes it from `Cargo.lock`, the crates' own source in the cargo registry and the ffmpeg libraries; the package step runs it, and it runs alone too. `scripts\notices.ps1 -Check` fails, naming the crate, when a crate's license is not on the allow list at the top of the script. today it names ffmpeg-sys-next, which is wtfpl.

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
