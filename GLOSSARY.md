# glossary

**loop analysis**:
the mode that plays a clip's last few seconds into its first few, over and over, so a bad loop point is obvious. the owner's name for it.
_Avoid_: seam mode, loop check, seam analysis

**seam**:
the single cut inside loop analysis where the last frame meets the first. the trip back from the first seconds to the last is not a seam; it is shown as a short hold on black.
_Avoid_: loop point, wrap

**gate**:
one pass or fail question that phase 0 answers on real hardware before any player code is written. listed in `docs/phase-0.md`.
_Avoid_: spike test, benchmark, check

**look-only**:
how the browser treats a network folder: it lists names, sizes and dates from the folder listing and opens nothing until the user opens a file or asks for previews.
_Avoid_: read-only (that is about never writing), lazy mode, offline mode

**poster**:
the one still frame shown on a browser card before anything else exists for that file.
_Avoid_: thumbnail

**hover previews**:
the 24 small frames per file that let a browser card scrub under the pointer. made the first time a card is hovered, then kept in the cache.
_Avoid_: strip, sprite, thumbnails, storyboard

**timebar strip**:
the small frames of the clip on screen made ready for the timebar's picture, once its first frame is up: one every two seconds, at most 100, and only key frames in a clip with long groups. the nearest shows at once while the frame under the pointer is read. the design's "denser strip" (section 06).
_Avoid_: strip alone (that is the row of tabs), hover previews (those are a card's), thumbnails, filmstrip

**played folder**:
the clips of one folder that a tab plays one after another, in name order. the loop mode all goes round it.
_Avoid_: playlist, queue, bin

**kept frames**:
the budgeted set of decoded frames held for a long-gop clip (h.264, hevc), inside which stepping back is instant.
_Avoid_: ring, gop cache, frame cache

**navigator**:
the small plan of the whole picture that stands at the lower left while the picture is zoomed past the window, the part in view marked on it. a press or a drag in it pans.
_Avoid_: minimap, overview, viewer, thumbnail

**address**:
the line across the top of the browser that names every folder from the drive or share down to the one shown. a press on a folder goes to it; a press on the rest of the line, or control + L, makes it a field a path is typed or pasted into.
_Avoid_: address bar, path bar, breadcrumbs (the bar is the control bar and nothing else)

**chip**:
a small label that stays on the picture while the view differs from the plain clip: zoom, speed, loop analysis, mirror, a single channel.
_Avoid_: badge, toast, pill

**toast**:
a short message that appears and goes away by itself, such as "reloaded".
_Avoid_: chip, notification, banner

**banner**:
a line across the top of the picture that states a problem and stays until it is resolved or dismissed, such as a lost file or missing sequence frames.
_Avoid_: toast, error dialog

**markup**:
shapes and text drawn on one frame of one clip, to be saved as an image. marks last until the tab closes.
_Avoid_: annotation, notes, review comments

**the bar**:
the single row of controls and the timebar at the bottom of the window, shown on hover.
_Avoid_: toolbar, controls overlay, transport

**the strip**:
the row of tabs and window buttons at the top of the window, shown on hover. it is also the drag handle for the window.
_Avoid_: title bar, tab bar, header

**release manifest**:
`eyepiece-release.txt`, the plain `key = value` file every release carries beside its files: the version, the date, a line for each change, and each file with its size and sha-256. the update learns what a release is from it and nothing else (adr 0007).
_Avoid_: manifest alone (cargo's is Cargo.toml), checksums, SHA256SUMS

**set aside**:
what an update does with an installed file it replaces: renamed `<name>.old-<version>` beside it, since a running program's files cannot be written over, and removed by its exact name at the next start or the next try. a download that failed is set aside too, as `<name>.failed` in the update's folder, which only that file's next failure replaces.
_Avoid_: backup, deleted, moved to trash

**panel**:
settings or help, drawn inside the window over whatever is up: a sidebar of pages and the page, 760 × 500 at the design's scale. the picture behind darkens, the bar steps aside and the strip stays. esc, its close mark, its button on the strip or a press outside it closes it.
_Avoid_: dialog, modal (the update's modal is another thing), window, preferences

## Flagged ambiguities

- **read-only** means the app never writes to media. **look-only** means the app does not even open files on a network folder. a local folder is read-only but not look-only.
- **strip** alone is the row of tabs at the top of the window. the design's section 06 also calls the frames made ready for the timebar a strip: that is always the **timebar strip**.
- **panel** alone is settings or help. the code and the design also say "the metadata panel" for the file's facts over the bar: that one is always named in full.
