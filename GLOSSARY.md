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

**kept frames**:
the budgeted set of decoded frames held for a long-gop clip (h.264, hevc), inside which stepping back is instant.
_Avoid_: ring, gop cache, frame cache

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

## Flagged ambiguities

- **read-only** means the app never writes to media. **look-only** means the app does not even open files on a network folder. a local folder is read-only but not look-only.
