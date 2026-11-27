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
the clips of one folder that a tab plays one after another, in name order; played with its subfolders, then each subfolder's in name order, depth first. the loop mode all goes round it. with the setting on, its lone stills are among its clips, each held for a set time while play runs (a held still).
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

**trim**:
the part of a clip from its in point to its out point, both frames kept: in 55, out 129 is 75 frames. loop one plays only the trim. it belongs to its tab's clip, is set by keys alone and is saved nowhere: it goes when the tab closes or another clip takes its place.
_Avoid_: selection, range, in-out, cut (a cut is where a played folder goes from one clip to the next)

**in point**:
the first frame of a trim, set at the frame on screen. with none set, the trim starts on the clip's first frame.
_Avoid_: mark in, start point

**out point**:
the last frame of a trim, set at the frame on screen, and kept as an editor's mark is. with none set, the trim ends on the clip's last frame.
_Avoid_: mark out, end point

**chip**:
a small label that stays on the picture while the view differs from the plain clip: zoom, speed, loop analysis, a trim, mirror, a single channel.
_Avoid_: badge, toast, pill

**toast**:
a short message that appears and goes away by itself, such as "reloaded".
_Avoid_: chip, notification, banner

**banner**:
a line across the top of the picture that states a problem and stays until it is resolved or dismissed, such as a lost file or missing sequence frames.
_Avoid_: toast, error dialog

**markup**:
shapes and text drawn on a clip, and the mode they are drawn in. each is a mark; marks are kept in groups, and a clip's groups are its notes.
_Avoid_: annotation, review comments

**group**:
a range of frames of a clip, in to out with the out frame kept as a trim's is, and the marks that show on every frame of it, while playing too. every mark belongs to one. a thin bar on the timebar, which a drag resizes and moves.
_Avoid_: annotation group, layer, selection, range alone (a group has a range; it is not one)

**notes**:
a clip's groups of marks as they are kept between runs, in its folder's `eyepiece.yaml`.
_Avoid_: annotations, comments, markup (that is the drawing and its mode), note for a toast

**eyepiece.yaml**:
the one file in a folder that holds the notes on the folder's clips, and for each a log of the exports made from it (adr 0010). eyepiece's own file, not media: it is read again before each save, written whole and swapped in, and its last version kept as `eyepiece.yaml.previous`. notes that cannot be beside their clip are kept the same way in eyepiece's own folder, where its settings are.
_Avoid_: sidecar, project file, metadata file

**the bar**:
the single row of controls and the timebar at the bottom of the window, shown on hover.
_Avoid_: toolbar, controls overlay, transport

**the strip**:
the row of tabs and window buttons at the top of the window, shown on hover. it is also the drag handle for the window.
_Avoid_: title bar, tab bar, header

**release manifest**:
`eyepiece-release.txt`, the plain `key = value` file every release carries beside its installer: the version, the date, a line for each change, and the installer with its size and sha-256. the update learns what a release is from it and nothing else (adr 0007, 0008).
_Avoid_: manifest alone (cargo's is Cargo.toml), checksums, SHA256SUMS

**set aside**:
what the update does with a download that failed: renamed `<name>.failed` in the update's folder, which only that file's next failure replaces. (until 0.3.0 the update also set aside each installed file it replaced, as `<name>.old-<version>`; the installer does that work now, adr 0008.)
_Avoid_: backup, deleted, moved to trash

**panel**:
settings or help, drawn inside the window over whatever is up: a sidebar of pages and the page, 760 × 500 at the design's scale. the picture behind darkens, the bar steps aside and the strip stays. esc, its close mark, its button on the strip or a press outside it closes it.
_Avoid_: dialog, modal (the update's modal is another thing), window, preferences

## Flagged ambiguities

- **read-only** means the app never writes to media. **look-only** means the app does not even open files on a network folder. a local folder is read-only but not look-only.
- **strip** alone is the row of tabs at the top of the window. the design's section 06 also calls the frames made ready for the timebar a strip: that is always the **timebar strip**.
- a **mark** is markup drawn on a clip, shown over its group's range. the two **markers** on the timebar are the trim's in point and out point, and are not marks; the thin **bars** over the track are the groups.
- a **trim** is the part of a clip that plays in loop one. a group's **range** is the frames its marks show on. shift with [ and ] set the trim's points, and a group's in and out only in markup mode with the group selected.
- **panel** alone is settings or help. the code and the design also say "the metadata panel" for the file's facts over the bar: that one is always named in full.
