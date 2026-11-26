<img src="assets/icon/eyepiece.png" alt="the eyepiece icon" width="128">

# eyepiece

a video review player for video professionals: extremely fast, lightweight, and only what you need.

[a tridant system](https://tridant.io)

![eyepiece playing a milky way time-lapse, tabs above, controls below](docs/images/hero.png)

## why eyepiece

- **it opens at once, playing:** a 4K clip is on screen 203 ms (hap q) or 269 ms (h.264) after launch. medians of twelve launches on a 10-core Windows development machine, 2026-11-24.
- **true frames and pixels.** a step is one frame of the file. at 100%, one video pixel is one display pixel.
- **exact or labeled.** metadata comes from the file. anything assumed is labeled.
- **no silent changes.** speed, zoom, mirror or a single channel leaves a chip on the picture.
- **only what you need.** one window. no account, no library, no project files. it never writes beside your media.

## get it

download the [latest release](https://github.com/tridant-io/eyepiece/releases/latest) from GitHub.

**Windows:** download `eyepiece-<version>-windows-setup.exe` and run it. it installs for the current user only, with no admin prompt, into `%LOCALAPPDATA%\Programs\eyepiece`, and adds a Start menu entry. uninstall it from Windows Settings > Apps. it updates itself: a newer release shows an **update** button, which says what changed and installs it. it needs 64-bit Windows 11 and a GPU with Direct3D 11.

**macOS:** download `eyepiece-<version>-macos.dmg`, open it and drag eyepiece to Applications. it is signed with an Apple Developer ID and notarized by Apple, and needs a Mac with Apple silicon and macOS 13 or later. it does not update itself yet: its **update** button opens the new release, downloaded the same way. this is the first macOS release, and it has been tried on macOS 26 only.

## quick start

1. **open:** drop a file or folder on the window, or press **O** for the browser.
2. **play:** **space** or a click plays and pauses.
3. **step:** **F** and **G**, or **←** and **→**. **shift** with an arrow steps ten. the wheel scrubs.
4. **fullscreen:** **P** or a double-click. **esc** leaves.
5. **controls** show when the pointer moves. the pin at the left of the bar keeps them up.
6. **find a key:** **?** for help, or right-click for every command and type to find one.

## features

### the window

- **the strip** on top holds the tabs, help and settings. drag it to move the window.
- **the bar** at the bottom holds first frame, step and play, the readout, the menus, metadata, screenshot, sound and the timebar.
- **chips** stay on the picture while it differs from the file.
- the window takes the clip's shape, up to 90% of the screen.
- **one window:** a file opened while eyepiece runs, or dropped on it, gets a tab of its own.

### playing and stepping

- **J K L** shuttle, each press of J or L doubling the speed up to 8×. **speed** runs from 0.1× to 8×.
- **R** turns looping off and on. a file loops by default.
- **T** switches frames and timecode. timecode is the file's own, drop-frame included, or counts from zero.
- **a file that changes on disk**, such as a new render, takes the old one's place on the same frame.

### the timebar

click or drag to go to a frame. hover for a picture of the frame under the pointer. ticks mark frames with markup, and missing frames of a sequence are amber.

![hovering the timebar shows that frame's picture and number](docs/images/timebar.png)

### zoom, pan and the navigator

- **0** fits, **1** is 1:1, **2** is 2×. **-**, **=** and **shift+wheel** zoom about the pointer, up to 6400%.
- the zoom chip names the pixel under the pointer and its values. from 1000%, a grid marks every pixel.
- drag or **alt+arrows** to pan. **the navigator** at the lower left shows the whole picture: press in it to go there.

![the picture at 1200% with pixel grid, zoom chip and navigator](docs/images/zoom-navigator.png)

### guides

**D** shows a grid of tiles and **C** the center lines and circle. they are drawn on the picture, so they stay true zoomed or letterboxed. a screenshot can carry them.

![the view menu with grid and center guides drawn](docs/images/guides.png)

### looking at the picture

- the view menu shows one channel (r, g, b or alpha) as gray. **H** mirrors, **V** flips, **Q** turns a quarter. each leaves a chip and never changes the file.
- deep files go to the screen as 16-bit. values above 1.0 stop at white: there is no tone map.

### metadata

rest the pointer on the metadata button to read it. click it or press **I** to pin it. it covers the file, codec, size, frames, rate, bit depth, chroma, color, alpha, audio, timecode and key frames. a value the file does not state is dim, with the assumption in use. faults, such as missing frames or a variable rate, are amber.

![the metadata panel pinned at the top right](docs/images/metadata.png)

### loop analysis

**A** plays the clip's last seconds into its first, over and over, so the seam is the only cut you see. the chip gives the seam's change, or warns in amber that the last and first frames are the same: the doubled frame that makes a loop stutter.

![loop analysis with its seam chip and the loop menu](docs/images/loop-analysis.png)

### tabs and comparing versions

- **N** or **+** opens a tab, **[** and **]** go round them, **ctrl+W** closes one and **ctrl+shift+T** brings it back.
- **compare versions:** open `wall_v011.mov` and `wall_v012.mov` in two tabs and flip with **[** and **]**. clips of one size and length switch on the same frame, zoom and pan.

### played folders

a folder's clips, played in name order in one tab. press **open folder** in the browser, or drop a folder on the window.

- **R** goes round loop all, one and off. **ctrl+←** and **ctrl+→** go to the clip before and after.
- click the tab to list the clips. a size or rate unlike the first clip's is amber.
- nothing jumps at a cut: the window is sized once and the next clip is ready.
- **play with subfolders** (right-click a folder, or **ctrl+shift+enter**) plays its own clips, then each subfolder's, depth first. the list names each by its path from the folder. it stops at 2000 videos or 10000 folders, and says so.

![a played folder's clip list, odd rates and size in amber](docs/images/played-folder.png)

### the browser

**O** lays the browser over the tab and **esc** goes back.

- **the sidebar** lists places, volumes and recent folders.
- **columns** lead to the folder. its videos are **cards**: move the pointer across one to scrub its preview.
- the keys act in one column or in the cards, as in finder's column view. what is selected there is lit; the folders on the way are dim.
- **the address** shows the path. press it, or **ctrl+L**, to type one.
- the two icons at its right **sort** the folder by name, size, date modified or created (and on a mac, date added), either way, folders first; and switch between **list**, **gallery** and both.
- **shift+click** or **ctrl+click** picks several videos to play as a played folder.
- **right-click** a folder, a video or the empty area for what can be done with it: open, play, reveal, copy path and more.
- **network folders are look-only:** names, sizes and dates, until you open a file or press **make previews**.
- previews are cached in the system's cache folder, never near your media.

![the browser: address, folder columns, cards and metadata column](docs/images/browser.png)

### screenshots and markup

- **S** saves the file's frame at full size as a png, 16-bit when the file is deeper. **shift+S** saves the view as shown. **ctrl+C** copies the frame.
- the screenshot menu can add a caption, the guides and the markup.
- screenshots go to `eyepiece` in your pictures folder, never beside the media or over an earlier one. the note that says where has **reveal in explorer** (finder on macOS), and stays up while the pointer is on it.
- **markup:** **E** pauses and shows the tools: rectangle, circle, pen, text and four inks. in a text field, **enter** keeps and **esc** drops. **ctrl+Z** takes back a mark. marks stay on their frame until the tab closes.

![markup on a frame: rectangle, circle, text and the tools](docs/images/markup.png)

![a saved screenshot with its marks and caption](docs/images/saved-frame.png)

### sound

the clip's sound plays with it and keeps the clock. pitch holds from half to double speed. beyond that, in reverse and in loop analysis, sound is off and a chip says so. **M** mutes, **↑** and **↓** set the volume.

### settings, help and about

the gear, or **ctrl+,**, opens settings. a change takes effect at once. settings are also a plain text file you can edit. **?** opens help: the keys, and what eyepiece does.

![the settings panel on its general page](docs/images/settings.png)

![the about page](docs/images/about.png)

## keys

on a Mac, command does what control does.

| | key | does |
|---|---|---|
| **play** | space | play / pause |
| | F · G, or ← · → | previous / next frame |
| | shift+F · shift+G, or home · end | first / last frame |
| | shift+← · shift+→ | ten frames back / forward |
| | J · K · L | reverse · stop · forward (J and L double the speed each press) |
| | wheel | scrub, one frame a notch |
| **zoom** | shift+wheel | zoom at the pointer |
| | 0 · 1 · 2 | fit · 1:1 · 2× |
| | - · = | zoom out / in |
| | double-click | fullscreen |
| | alt+arrows | pan |
| **tabs** | O | open: the browser over this tab |
| | N | new tab |
| | [ · ] | previous / next tab |
| | ctrl+← · ctrl+→ | previous / next clip of the folder |
| | ctrl+W | close tab |
| | ctrl+shift+T | reopen closed tab |
| **loop, speed** | R | loop off · one · all |
| | A | loop analysis |
| | shift+- · shift+= | slower / faster |
| | backspace | 1× speed |
| **view** | T | frames / timecode |
| | I | pin metadata |
| | D · C | grid · center lines |
| | H · V · Q | mirror · flip · rotate |
| **screenshot** | S | screenshot of the frame |
| | shift+S | screenshot of the view |
| | ctrl+C | copy frame |
| | E | markup mode |
| | ctrl+Z | take back a mark |
| **sound** | M | mute |
| | ↑ · ↓ | volume up / down |
| **window** | P | fullscreen |
| | shift+P | fullscreen on the next display |
| | right-click | every command |
| | ? | help, on its page of keys |
| | ctrl+, | settings |
| | esc | leave fullscreen, close |

**esc** leaves fullscreen, then drops markup text being typed, then closes a menu, then ends markup mode. with help up, any key closes it and then does what it does. **esc** and **?** only close it.

in the browser, letters select names instead of running commands:

| key | does |
|---|---|
| ↑ · ↓ | move in the column. with nothing selected, ↓ takes the first and ↑ the last |
| → · ← | into a folder, on what was selected there, else its first; from a video, into the cards · back to the column before |
| arrows, in the cards | card by card, over the ends of rows; ↑ · ↓ a row. ← from the first card goes back to the column |
| home · end | first / last |
| enter | open the selected card or folder, or play the videos picked |
| shift+enter | open folder: play the folder |
| ctrl+shift+enter | play the folder with its subfolders |
| backspace | up a folder |
| letters | select the first name that starts with them (a second's pause starts over) |
| ctrl+L | type or paste a path into the address |
| alt+← · alt+→ | back / forward (also the keyboard's back and forward keys and the mouse's side buttons) |
| f5 · ctrl+R | read again |
| esc | back to the clip |

## formats

| | |
|---|---|
| **video** | hap (hap, hap alpha, hap q, hap q alpha, hap alpha-only, hap r, hap hdr), notchlc, prores (422 proxy to 4444 xq), h.264, hevc (8 and 10-bit), vp9 |
| **containers** | mov, mp4 (fragmented too), mkv, webm |
| **stills** | png, jpeg, tiff, tga, dpx, exr: one file, or a numbered sequence as one clip |
| **sound** | the clip's own sound track |
| **decoding** | hap on the GPU. h.264, hevc and vp9 on the GPU where it can, else in software. prores and notchlc in software. on macOS, h.264, hevc and prores 422 use the system's decoder |
| **untested** | hap q alpha and hap alpha-only on real files. exr compressions other than none, rle and zip |
| **not yet** | mxf, mpeg-ts and other containers that do not list their frames up front |

## privacy

- **the update check is all eyepiece sends.** at most once a day it asks GitHub for the latest release's manifest, with only `eyepiece/<version>` as its user agent: no account, id, file name or setting. a switch in settings turns it off, and a release is downloaded only when you press **update**.
- on a network volume, the browser reads only the folder's listing until you open a file or ask.
- what eyepiece keeps stays on your machine: its settings file, the preview cache and your screenshots.

## build from source

needs Windows, Rust (stable, msvc target) and the Visual Studio C++ build tools. the tests need a GPU with a video decoder.

```powershell
# once: downloads the LGPL FFmpeg shared build and libclang into third_party
powershell -NoProfile -ExecutionPolicy Bypass -File scripts\setup.ps1

cargo build --release
cargo run --release -- wall_v012.mov
cargo test
```

for the end-to-end tests, packaging, the macOS build and everything else, see [docs/development.md](docs/development.md).

## license

eyepiece is open source under the [Common Public Attribution License 1.0](LICENSE) (CPAL-1.0). Copyright (c) 2026 Tridant, Inc.

every copy and derivative must show, in its user interface each time it runs, the notice "Copyright (c) 2026 Tridant, Inc.", the phrase "a tridant system", the tridant mark and https://tridant.io (Exhibit B of the [LICENSE](LICENSE)). removing the attribution, or crediting another party more prominently, breaks the license.

"eyepiece", "tridant", "a tridant system" and the tridant mark are trademarks of Tridant, Inc., which the license does not cover: see [TRADEMARKS.md](TRADEMARKS.md).

third-party notices ship with the program: FFmpeg under the LGPL, as separate libraries you may replace; the Rust crates, each with its license; and the typefaces IBM Plex Mono and Archivo under the SIL Open Font License.

## credits

eyepiece is built by [tridant](https://tridant.io). the screenshots show six-second excerpts, without sound and re-encoded, of:

- [Moon Phase and Libration, 2026](https://svs.gsfc.nasa.gov/5587/): NASA's Scientific Visualization Studio, Ernie Wright, Noah Petro and James Tralie. public domain.
- [Thermonuclear Art](https://svs.gsfc.nasa.gov/12034/): NASA's Goddard Space Flight Center; the SDO Team, Genna Duberstein and Scott Wiessinger, producers. public domain.
- [Zoom into Webb's View of the Pillars of Creation](https://esawebb.org/videos/weic2216d/): NASA, ESA, CSA, and STScI, ESO, NOIRLab/NSF/AURA, T.A.Rector, B.A.Wolpa, ESA/Hubble, J. DePasquale, A. Koekemoer, A. Pagan, N. Bartmann, M. Zamani. [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).
- [Dusk till dawn over Paranal](https://www.eso.org/public/videos/uhd_cm_paranal_3/): ESO / C. Malin. [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

none of them endorses eyepiece or tridant.
