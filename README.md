# eyepiece

a review player for vfx and install content, by tridant. it opens a clip at once, steps one true frame at a time, shows true pixels, and says exactly what the file is.

**status: phase 0.** nothing plays yet. this repo holds the approved design and the gate tests that decide whether the design's load-bearing claims hold on real hardware.

## what is here

| path | what |
|---|---|
| `docs/design/plan.html` | the approved design, rev c (2026-11-23). open it in a browser. |
| `docs/phase-0.md` | the gates: what each proves, how it is measured, and the results |
| `docs/adr/` | decisions that are hard to reverse, one file each |
| `GLOSSARY.md` | the project's words |
| `spike/` | throwaway gate programs. not the player. |

## run the gates (windows)

needs rust (stable, msvc target) and the visual studio c++ build tools.

```powershell
# fetches an lgpl shared ffmpeg and a libclang into spike\third_party (ignored by git)
powershell -NoProfile -File spike\setup.ps1
# then, for any gate listed in docs/phase-0.md
cargo run --manifest-path spike\Cargo.toml --release --bin <gate>
```

each gate prints its measurements and ends with `PASS` or `FAIL`.

## deploy

nothing ships yet. when it does, every build is signed: windows on the dev machine, macos on tec-mba. the signing gate in `docs/phase-0.md` confirms both before any player code is written.
