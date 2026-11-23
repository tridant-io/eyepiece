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
# once: fetches an lgpl shared ffmpeg and a libclang into spike\third_party (ignored by git)
powershell -NoProfile -ExecutionPolicy Bypass -File spike\setup.ps1
# once: makes the test clips in spike\media (about 10 GB with -Big; needs a full ffmpeg on the path for its encoders)
powershell -NoProfile -ExecutionPolicy Bypass -File spike\media.ps1 -Big
# then any gate, for example
powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate.ps1 gate1_zerocopy spike\media\hevc10_4k_5994.mp4
```

the gates are `gate1_zerocopy`, `gate2_present`, `gate3_ramp`, `gate4_coldstart`, `gate5_share`, and the measurements `hap_demux`, `hap_speed`, `hap_play`, `decode_speed`, `devtime`. a gate ends with `PASS` or `FAIL`; a measurement only prints numbers. what each one found is in `docs/phase-0.md`.

windows only so far. the macos side of each gate is not written yet.

## deploy

nothing ships yet. when it does, every build is signed: windows on the dev machine, macos on tec-mba. the signing gate in `docs/phase-0.md` confirms both before any player code is written.
