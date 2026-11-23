# eyepiece

a review player for vfx and install content. the design is `docs/design/plan.html`; what the owner settled is its section 10. read it before changing behavior.

use the terms in `GLOSSARY.md` and read the ADRs in `docs/adr/` for the area you touch.

## rules the code can't show

- **media is read-only.** never write into or beside a media file, never delete, rename or move one. files are opened shared (read, write and delete sharing) so a render can overwrite a clip that is open.
- **network volumes are look-only.** on a network path, use the folder listing and nothing else: no open, no stat call that opens, no probe, until the user opens that file or asks. unknown volume types count as network. tests and gates never touch the owner's nas except in a scratch folder they named.
- **the cache** lives only in the os cache folder, in its own `eyepiece` folder. eviction removes one cache file at a time, never a directory, never recursively, and refuses to run if its root resolves to nothing, a drive root or a home folder.
- **no number without a measurement.** a speed or size figure in docs or ui copy is either measured (say where) or called a target.
- **exact or labeled.** a metadata value comes from the file, or the ui says the file does not carry it and shows the assumption in use. never trust ffmpeg's summary fields for frame count, frame rate, range or color tags (adr 0003, plan section 05).

## ui rules

- copy is lowercase, american spelling: color, center.
- the owner's keys are fixed: F/G step, shift+F/G first/last, O open, N new tab, [ ] tabs, 0/1/2 fit/1:1/2x, P fullscreen, esc. add aliases, never move these.
- no accent color on the tab strip or the control bar. the accent (orange) is only for sliders, switches and the focused field inside menus. amber marks a fault in a file and nothing else.
- anything that changes what you see (speed, zoom, mirror, a single channel) leaves a chip on screen.

## working here

- `spike/` is throwaway. it answers the gates in `docs/phase-0.md` and is deleted when the player starts.
- commits: `type: details`, lowercase, one line. types: fix, feat, chore, docs, refactor, test, release.
- a phase is done when its build is installed and checked on the owner's machine, not when it passes here.
