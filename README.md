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
- **only what you need.** one window. no account, no library, no project files. it writes beside your media only when you save a trim, and never over a file.

## get it

download the [latest release](https://github.com/tridant-io/eyepiece/releases/latest) from GitHub.

**Windows:** download `eyepiece-<version>-windows-setup.exe` and run it. it installs for the current user only, with no admin prompt, into `%LOCALAPPDATA%\Programs\eyepiece`, and adds a Start menu entry. uninstall it from Windows Settings > Apps. it adds eyepiece to Explorer's **Open with** for mov, mp4, m4v, mkv and webm files, and makes it the default for none of them: to do that, settings > general > **make eyepiece the default for videos** opens eyepiece's page in Windows' Default apps, where you choose. it updates itself: a newer release shows an **update** button, which says what changed and installs it. it needs 64-bit Windows 11 and a GPU with Direct3D 11.

**macOS:** download `eyepiece-<version>-macos.dmg`, open it and drag eyepiece to Applications. it is signed with an Apple Developer ID and notarized by Apple, and needs a Mac with Apple silicon and macOS 13 or later. Finder lists it under **Open With** for mov, mp4, m4v, mkv and webm files and leaves the default as it is: to make eyepiece the default for a kind of video, select one in Finder, choose **Get Info**, pick eyepiece under **Open with** and press **Change All**. it does not update itself yet: its **update** button opens the new release, downloaded the same way. this is the first macOS release, and it has been tried on macOS 26 only.

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
- **the bar** at the bottom holds first frame, step and play, the readout, the menus, metadata, screenshot, sound and the timebar, and the trim's button while a trim is set. an image sequence adds its frame rate, beside the speed. in a narrow window nothing on it overlaps: as room runs out the readout goes, the loop shows its icon alone, the rate its number, the speed hides while it is 1× (shift+- and shift+= still set it), and the zoom shows its number. for a still it holds only the zoom, the view, metadata, screenshot and fullscreen; a still that a played folder holds for a time adds play, the loop, and a timebar and readout in seconds.
- **chips** stay on the picture while it differs from the file.
- the window takes the clip's shape, up to 90% of the screen.
- **one window:** a file opened while eyepiece runs, or dropped on it, gets a tab of its own.

---

### playing and stepping

- **J K L** shuttle, each press of J or L doubling the speed up to 8×. **speed** runs from 0.1× to 8×.
- **R** turns looping off and on. a file loops by default.
- **T** switches frames and timecode. timecode is the file's own, drop-frame included, or counts from zero.
- **a file that changes on disk**, such as a new render, takes the old one's place on the same frame.
- **an image sequence's rate:** a numbered run of stills plays at its files' own rate when they state one (exr), else at the default for sequences: 24, or what settings > playback > image sequences > **play at** says. the rate button beside the speed reads the rate and where it comes from: `24 fps assumed` for the default, `25 fps set` for one chosen for this sequence, the rate alone for the files' own (a narrow window shows the rate alone). its menu has 23.976, 24, 25, 29.97, 30, 50, 59.94 and 60, the files' own first when they state one, and a field for any other: a number (12, 47.952) or a ratio (30000/1001), 1 to 240. 23.976, 29.97 and 59.94, chosen or typed, are exactly 24000/1001, 30000/1001 and 60000/1001.
- a rate chosen on the bar is that sequence's alone, and holds at once: the frame stays and play goes on. its timecode counts at the new rate; its frames, trim, markers and notes stay where they are. the speed multiplies whatever the rate is. the title and the metadata panel's **plays at** say the rate and where it comes from too.

---

### the timebar

![hovering the timebar shows that frame's picture and number](docs/images/timebar.png)
*hovering the timebar shows that frame's picture and number*

click or drag to go to a frame: while playing, play goes on from where you let go. hover for a picture of the frame under the pointer. a thin bar over the track stands for each group of marks (see markup), and missing frames of a sequence are amber.

- **trim:** **shift+[** sets the in point at the frame on screen and **shift+]** the out point. the out frame is kept: in 55, out 129 is 75 frames. a point alone trims to the clip's end or start. **shift+backspace**, or **clear** in the trim's menu, clears both.
- two markers stand on the timebar and the track outside the trim is dimmed. a chip names the trim, and the hover picture gives a frame's place in it.
- **drag a marker** to move its point. with the pointer on one it lights, and the pointer turns to a two-way arrow. a press takes it and goes nowhere in the clip; it then follows the pointer a frame at a time, and stops at the other marker. the clip takes the new trim when you let go, and stays on the frame it was on. a marker taken while playing holds play until you let go.
- on a marker, the box over the timebar names its point, its frame and the frames the trim keeps (`in 000055 · 75 f`), and its picture is the very frame it trims to, in h.264 and hevc too: the nearest picture had shows until that frame is read.
- where a group's bar passes over a marker, a press on the bar is the group's and a press on the track under it is the marker's.
- with a trim set, loop one plays only the trim, and loop analysis plays its out into its in. steps and the timebar still reach every frame. loop off plays the whole clip, as does loop all on its way through a played folder.
- a trim's in and out belong to its clip and are kept nowhere: they go when the tab closes or another clip takes its place. a still has none. a copy of the trim is saved with its button, below.

---

### saving a trim

- with a trim set, a **trim** button stands on the bar, left of the loop. its menu says what the trim keeps, saves it and clears it.
- a copy moves the file's own frames and sound into a new file of the same kind: nothing is re-encoded. where every frame is a key frame (prores, hap, notchlc) it is exactly the trim, and the menu says nothing more. with long groups (h.264, hevc) it starts at the key frame at or before the in point, and where frames are stored out of order it can end a frame or two past the out point; the menu names the frame it starts on and how many frames it keeps. an exact cut of those needs an encode, which is not built yet. image sequences are not saved yet.
- **save new file** writes `<name>_trim.mov` beside the clip, never over a file: then `<name>_trim 2.mov`, and so on. **keep sound** off leaves the sound out: `<name>_trim_nosound.mov`.
- **replace original…** asks first. the copy is written and checked, then the original goes to the recycle bin (the trash on macOS) and the copy takes its name, and the tab shows it. the original is never deleted: replace is not offered where it could not go to a bin (a network volume, which has none; a bin that is off, or smaller than the file; a drive whose bin Windows keeps no setting for), and Windows is stopped at anything it would delete rather than recycle. if the original cannot go, both are kept and the note says why.
- **save the trim without sound**, in the right-click menu, copies the trim without its sound, or with no trim set the whole clip: `<name>_nosound.mov`. it has no key.
- **include notes** (on at first, and kept between runs) gives the copy the clip's groups of marks on the frames it holds as notes of its own, in the folder's `eyepiece.yaml` (see [notes](#notes)): counted from the copy's first frame and cut to its length, a group wholly outside it left out. a copy cannot change the picture, so the marks are not drawn into it. off, the copy has no notes. in the clip's place, the new file's notes are those groups or none, as its frames changed; the groups the clip had are kept in `eyepiece.yaml.previous`, and the original is in the recycle bin. the right-click menu's **exclude notes from a copy** (or **include notes in a copy**) is the same switch, for a copy without sound with no trim set.
- every copy saved is logged on its clip in the folder's `eyepiece.yaml`: when, the new file's name, the clip's frames it holds (from the key frame it starts at) and how (`copy`, or `copy, no sound`). a copy in the clip's place is logged under the clip's name, which it takes. a copy that fails or is stopped is not logged. with **save notes beside the clip** off, or where the folder's file cannot be written, the log and the copy's notes are kept in eyepiece's own folder, as notes are.
- the copy keeps the timecode of its first frame (drop-frame stays drop-frame), the rotation, the color tags and the creation time. its sound is copied packet for packet, never cut inside one, so it can end up to one packet past the picture: 21 ms for aac at 48 kHz. other tracks (subtitles, data, a second picture) are not kept, and the menu and the note say so.
- it is fast: a whole 552 MB prores 422 hq clip without its sound took 0.78 s, written and on the disk, and 51 frames of it 0.30 s (measured on the Windows development machine, 2026-11-26, with the clip in the system's cache).
- before it says saved, eyepiece reads the copy back its own way: its frames, how long they last, its timecode and its sound. if one is not what was asked, the note says what differs. a copy that fails or is stopped is kept as `<name>.failed`, never deleted.
- a copy runs while you keep working. the note low in the window says how far it has got, and **stop** stops it. one copy is saved at a time.

---

### zoom, pan and the navigator

![the picture at 1200% with pixel grid, zoom chip and navigator](docs/images/zoom-navigator.png)
*the picture at 1200% with pixel grid, zoom chip and navigator*

- **0** fits, **1** is 1:1, **2** is 2×. **-**, **=** and **shift+wheel** zoom about the pointer, up to 6400%.
- the zoom chip names the pixel under the pointer and its values. from 1000%, a grid marks every pixel.
- drag or **alt+arrows** to pan. **the navigator** at the lower left shows the whole picture: press in it to go there.

---

### guides

![the view menu with grid and center guides drawn](docs/images/guides.png)
*the view menu with grid and center guides drawn*

**D** shows a grid of tiles and **C** the center lines and circle. they are drawn on the picture, so they stay true zoomed or letterboxed. a screenshot can carry them.

---

### looking at the picture

- the view menu shows one channel (r, g, b or alpha) as gray. **H** mirrors, **V** flips, **Q** turns a quarter. each leaves a chip and never changes the file.
- deep files go to the screen as 16-bit. values above 1.0 stop at white: there is no tone map.

---

### metadata

![the metadata panel pinned at the top right](docs/images/metadata.png)
*the metadata panel pinned at the top right*

rest the pointer on the metadata button to read it. click it or press **I** to pin it. it covers the file, codec, size, frames, rate, bit depth, chroma, color, alpha, audio, timecode and key frames. a value the file does not state is dim, with the assumption in use. faults, such as missing frames or a variable rate, are amber.

---

### loop analysis

![loop analysis with its seam chip and the loop menu](docs/images/loop-analysis.png)
*loop analysis with its seam chip and the loop menu*

**A** plays the clip's last seconds into its first, over and over, so the seam is the only cut you see. the chip gives the seam's change, or warns in amber that the last and first frames are the same: the doubled frame that makes a loop stutter.

---

### tabs and comparing versions

- **N** or **+** opens a tab, **[** and **]** go round them, **ctrl+W** closes one and **ctrl+shift+T** brings it back.
- **compare versions:** open `wall_v011.mov` and `wall_v012.mov` in two tabs and flip with **[** and **]**. clips of one size and length switch on the same frame, zoom and pan.

---

### played folders

![a played folder's clip list, odd rates and size in amber](docs/images/played-folder.png)
*a played folder's clip list, odd rates and size in amber*

a folder's clips, played in name order in one tab. press **open folder** in the browser, or drop a folder on the window.

- **R** goes round loop all, one and off. **ctrl+←** and **ctrl+→** go to the clip before and after.
- every clip starts from its first frame, whether play runs on to it or you step to it.
- click the tab to list the clips. a size or rate unlike the first clip's is amber.
- nothing jumps at a cut: the window is sized once and the next clip is ready.
- **play with subfolders** (right-click a folder, or **ctrl+shift+enter**) plays its own clips, then each subfolder's, depth first. the list names each by its path from the folder. it stops at 2000 videos or 10000 folders, and says so.
- a folder dropped on the window, or given to eyepiece at launch, plays with its subfolders the same way.
- **stills:** a played folder plays its videos and sequences. turn on settings > playback > **play stills in a played folder** and its lone stills play too, in name order with the rest: each holds for 5 seconds while play runs (**hold a still for** sets 0.5 to 60), then play goes on as at a clip's end. paused, a still stays. a still among files picked together in the browser holds the same way, whatever the setting.

---

### the browser

![the browser: address, folder columns, cards and metadata column](docs/images/browser.png)
*the browser: address, folder columns, cards and metadata column*

**O** lays the browser over the tab and **esc** goes back.

- **the sidebar** lists places, volumes and recent folders.
- **columns** lead to the folder. its videos are **cards**: move the pointer across one to scrub its preview. a path deeper than fits scrolls: each step along it slides the columns there, and the scrollbar under them, dragged or under the wheel, goes back up the path. a click on a folder back up the path leaves the columns under the pointer, and a moment later they slide along to fill the room it left.
- drag the edge of the sidebar, of a column or of the metadata column to resize it; a double click puts it back. the cards give the room, down to 72 wide, and their pictures shrink with them. while not all the columns fit, the cards have an edge of their own just inside their left: drag it right for narrower cards and more columns.
- the keys act in one column or in the cards, as in finder's column view. what is selected there is lit; the folders on the way are dim.
- **the address** shows the path. press it, or **ctrl+L**, to type one.
- the two icons at its right **sort** the folder by name, size, date modified or created (and on a mac, date added), either way, folders first; and switch between **list**, **gallery** and both.
- **shift+click** or **ctrl+click** picks several videos to play as a played folder.
- **shift+click** or **ctrl+click** on folders in a column picks several of them: the cards then show the videos of them all, folder after folder, and **open folder** plays them as one played folder. a plain click goes back to one folder. in the list view, which has no cards, the folders only light up.
- **right-click** a folder, a video or the empty area for what can be done with it: open, play, reveal, copy path and more.
- **network folders are look-only:** names, sizes and dates, until you open a file or press **make previews**.
- previews are cached in the system's cache folder, never near your media.

---

### screenshots and markup

![markup on a frame: rectangle, circle, text and the tools](docs/images/markup.png)
*markup on a frame: rectangle, circle, text and the tools*

![a saved screenshot with its marks and caption](docs/images/saved-frame.png)
*a saved screenshot with its marks and caption*

- **S** saves the file's frame at full size as a png, 16-bit when the file is deeper. **shift+S** saves the view as shown. **ctrl+C** copies the frame.
- the screenshot menu can add a caption, the guides and the markup.
- screenshots go to `eyepiece` in your pictures folder, never beside the media or over an earlier one. the note that says where has **reveal in explorer** (finder on macOS), and stays up while the pointer is on it.
- **markup:** **E** pauses and shows the tools: select, rectangle, circle, pen, text, and ten inks: red, orange, yellow, green, blue, indigo, violet, white, black and gray. hold **shift** while drawing for a square or a round circle, as you see it; press or let go of it mid-drag. pen strokes are smoothed a little as you draw; settings > screenshots sets how much, or turns it off. in a text field, **enter** keeps and **esc** drops.
- **groups:** every mark belongs to a group, and a group shows over a range of frames, while the clip plays too. a mark drawn on a frame no group covers starts a group there that lasts 3 seconds (settings > screenshots, 0.5 to 60) and no further than the clip's end; a still's group is its one frame. a mark drawn on a frame a group covers joins it: the group selected when its range holds the frame, else the newest that does. the group a mark went into is selected.
- **ranges:** on the timebar each group is a thin bar just over the track, a lane higher where it overlaps another. a click on a bar selects its group, and a chip names its range. drag an end of the bar to move the group's in or out, and the rest of it to move the whole range. in markup mode with a group selected, **shift+[** and **shift+]** set its in and out at the frame on screen; with none selected, and outside markup mode, they set the trim's. the out frame is kept, as a trim's is. a mark's menu has **own range**, which moves the mark into a group of its own with the range it had. the tools' clear takes off the group selected, or with none every group on the frame on screen.
- **changing marks:** with select, a click selects a mark, a drag moves it and a drag of a corner handle scales it; **shift** keeps its proportion, and text always keeps its own. **delete** or **backspace** takes the selected mark off. a right click on a mark, with any tool, opens its menu: delete, edit text for text, own range, and its color. **ctrl+Z** takes back the last change to the clip's marks and ranges, one at a time, and **ctrl+shift+Z** or **ctrl+Y** brings it back, until a new change; the row of tools has both. **esc** lets go of the selected mark and group.

---

### notes

a clip's groups of marks are its notes, and they are kept. each folder gets one `eyepiece.yaml` that holds the notes on its clips and a log of the copies saved from them, and eyepiece reads it when a clip there opens. here a trim of 55 to 129, copied from the key frame at 48, took the group with it:

```yaml
# eyepiece's notes for the clips in this folder
version: 1
clips:
  pillars_v012.mov:
    file: { bytes: 578123456, modified: 2026-11-26T19:40:12Z }
    groups:
      - frames: [55, 129]
        marks:
          - { circle: [812, 440, 120, 120], ink: red }
          - { rect: [100, 100, 300, 200], ink: yellow }
          - { pen: [10, 10, 12, 14, 15, 20], ink: red }
          - { text: "this flickers", at: [900, 300], size: 1.0, ink: white }
    exports:
      - { when: 2026-11-26T19:52:03Z, file: pillars_v012_trim.mov, frames: [48, 129], how: copy }
  pillars_v012_trim.mov:
    file: { bytes: 97534122, modified: 2026-11-26T19:52:03Z }
    groups:
      - frames: [7, 81]
        marks:
          - { circle: [812, 440, 120, 120], ink: red }
          - { rect: [100, 100, 300, 200], ink: yellow }
          - { pen: [10, 10, 12, 14, 15, 20], ink: red }
          - { text: "this flickers", at: [900, 300], size: 1.0, ink: white }
```

- `frames` are a group's in and out, counted as the file counts, from 0. places are the file's pixels: a rectangle and a circle by left, top, width and height, a pen by its points, text by its upper left corner. inks go by name.
- `exports` logs each copy saved from the clip (see [saving a trim](#saving-a-trim)): when, in utc, the new file's name, the clip's frames it holds, and how it was made. a copy runs no ffmpeg command, so its line names none.
- a numbered run is one clip, named with a `#` for each digit (`name.####.exr`), and its frames are its files' numbers, so frames rendered later leave its notes where they are.
- `file` is the clip's size and when it was written. a clip that is another file now still shows its notes, with a chip that says they were made on an earlier version of the file, until you change them. a renamed clip leaves its notes under its old name.
- the file is there only while there is a note or a copy's log to keep. opening a clip never makes it. a clip whose last note is taken off loses its entry, and a file left with no clip in it is not deleted: it becomes `eyepiece.yaml.previous`.
- notes are saved a moment after a change, when a tab closes and when the window does, off the drawing thread.
- saving is careful. the file is read again before each save and only the clip saved is changed, so notes on other clips, another person's or another window's, stay. the new file is written whole beside the old one, flushed to the disk and read back, then swapped in in one step, and the version it replaced is kept as `eyepiece.yaml.previous`. two saves of the same clip at the same moment: the last one wins for that clip.
- a file eyepiece cannot read is never written over. eyepiece says which line it cannot read, and keeps that folder's notes in its own folder until it can.
- a folder that cannot be written (a read-only share, a camera card) keeps its notes in eyepiece's own folder, and eyepiece says so once. notes kept there are shown with those in the folder's file, and move into it at the first save that can be made there.
- network folders too: notes are saved beside the clips on a network volume as anywhere, so a team on a shared folder sees each other's notes.
- settings > screenshots has **save notes beside the clip**. off, notes are kept in eyepiece's own folder (`%APPDATA%\eyepiece\notes`, on macOS `~/Library/Application Support/eyepiece/notes`), a file for each folder of clips, saved with the same care.

---

### sound

the clip's sound plays with it and keeps the clock. pitch holds from half to double speed. beyond that, in reverse and in loop analysis, sound is off and a chip says so. **M** mutes, **↑** and **↓** set the volume.

---

### settings, help and about

![the settings panel on its general page](docs/images/settings.png)
*the settings panel on its general page*

![the about page](docs/images/about.png)
*the about page*

the gear, or **ctrl+,**, opens settings. a change takes effect at once. the playback page holds the rate image sequences open at when their files state none, for sequences opened after it changes. the screenshots page also holds markup's pen smoothing, how long a new group of marks lasts, and whether notes are saved beside the clip. settings are also a plain text file you can edit. **?** opens help: the keys, and what eyepiece does.

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
| **trim** | shift+[ · shift+] | set in / set out, at the frame on screen |
| | drag a marker on the timebar | move the in or out point |
| | shift+backspace | clear in and out |
| | trim button | save the trim: beside the clip, or in its place |
| **view** | T | frames / timecode |
| | I | pin metadata |
| | D · C | grid · center lines |
| | H · V · Q | mirror · flip · rotate |
| **screenshot** | S | screenshot of the frame |
| | shift+S | screenshot of the view |
| | ctrl+C | copy frame |
| | E | markup mode |
| | shift+[ · shift+] | in markup mode with a group selected: its in / out, at the frame on screen |
| | ctrl+Z | undo a change to marks |
| | ctrl+shift+Z · ctrl+Y | redo a change to marks |
| | delete · backspace | take off the selected mark (backspace is 1× speed when none is selected) |
| **sound** | M | mute |
| | ↑ · ↓ | volume up / down |
| **window** | P | fullscreen |
| | shift+P | fullscreen on the next display |
| | right-click | every command |
| | ? | help, on its page of keys |
| | ctrl+, | settings |
| | esc | leave fullscreen, close |

**esc** leaves fullscreen, then drops markup text being typed, then closes a menu, then lets go of a selected mark and group, then ends markup mode. with help up, any key closes it and then does what it does. **esc** and **?** only close it.

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
- what eyepiece keeps stays on your machine: its settings file, the preview cache and your screenshots. your notes are saved beside the clips they are on, in each folder's `eyepiece.yaml`, and go where the folder goes; a setting keeps them on your machine instead. a trim you save is written beside its clip, and logged in that folder's `eyepiece.yaml`.

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
