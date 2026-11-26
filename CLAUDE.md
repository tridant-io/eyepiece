# eyepiece

a video review player for video professionals: extremely fast, lightweight, and only what you need. the design is `docs/design/plan.html`; what the owner settled is its section 10. read it before changing behavior.

use the terms in `GLOSSARY.md` and read the ADRs in `docs/adr/` for the area you touch.

## rules the code can't show

- **media is read-only unless the user asks.** never write into a media file, never delete or rename one. trim and export write a new file, beside the source by default, never over an existing file. replacing the original is offered only behind a prompt: the new file is written and checked first, then the original goes to the recycle bin or trash, never deleted, so not on a network volume (the owner, 2026-11-26). files are opened shared (read, write and delete sharing) so a render can overwrite a clip that is open.
- **network volumes are look-only.** on a network path, use the folder listing and nothing else: no open, no stat call that opens, no probe, until the user opens that file or asks. unknown volume types count as network. tests and gates never touch the owner's nas except in a scratch folder they named.
- **the cache** lives only in the os cache folder, in its own `eyepiece` folder. eviction removes one cache file at a time, never a directory, never recursively, and refuses to run if its root resolves to nothing, a drive root or a home folder.
- **no number without a measurement.** a speed or size figure in docs or ui copy is either measured (say where) or called a target.
- **exact or labeled.** a metadata value comes from the file, or the ui says the file does not carry it and shows the assumption in use. never trust ffmpeg's summary fields for frame count, frame rate, range or color tags (adr 0003, plan section 05).

## ui rules

- copy is lowercase, american spelling: color, center.
- mono (ibm plex mono) only for titles and numbers: headings, values, file names and paths, key caps. every sentence, note, message, tooltip, button and menu word is archivo; a sentence with numbers in it is archivo throughout (the owner's word, 2026-11-24).
- the owner's keys are fixed: F/G step, shift+F/G first/last, O open, N new tab, [ ] tabs, shift+[ ] trim in and out, 0/1/2 fit/1:1/2x, P fullscreen, esc. add aliases, never move these.
- a trim keeps its out frame, as editors' marks do (in 55, out 129 is 75 frames), and loop one plays only the trim (the owner, 2026-11-26).
- no accent color on the tab strip or the control bar, not even for a button that is there only for a moment (trim). the accent (orange) lives only in menus, popups and panels: sliders, switches, the focused field, and the action a popup asks you to confirm (the owner, 2026-11-26). amber marks a fault in a file and nothing else.
- anything that changes what you see (speed, zoom, mirror, a single channel) leaves a chip on screen.

## working here

- `spike/` is throwaway. it answers the gates in `docs/phase-0.md`. it stays while the mac renderer still has gates to rerun, and is deleted after that with the owner's ok. the player never depends on it.
- the player is the crate at the repo root. `cargo test` and `cargo run` need `scripts\setup.ps1` once.
- commits: `type: details`, lowercase, one line. types: fix, feat, chore, docs, refactor, test, release.
- every release brings the readme up to date and retakes its screenshots (`scripts\shots.ps1`), before or in the release commit (the owner, 2026-11-26).
- a phase is done when its build is installed and checked on the owner's machine, not when it passes here.
