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
# gate 6 is its own script: it brings up a scratch samba share in a small wsl distro (eyepiece-gate6) and reads the share's log
powershell -NoProfile -ExecutionPolicy Bypass -File spike\gate6.ps1
```

the gates are `gate1_zerocopy`, `gate2_present`, `gate3_ramp`, `gate4_coldstart`, `gate5_share`, `gate6_browse`, and the measurements `hap_demux`, `hap_speed`, `hap_play`, `decode_speed`, `devtime`. a gate ends with `PASS` or `FAIL`; a measurement only prints numbers. what each one found is in `docs/phase-0.md`.

## run the gates (macos)

needs rust and apple's command line tools. the mac has no github access of its own: copy `spike/` there (the windows machine does it with tar over ssh).

```sh
# once: builds an lgpl shared ffmpeg 9.0 with videotoolbox into spike/third_party (ignored by git)
sh spike/mac/setup.sh
# then a gate
sh spike/mac/gate.sh gate5_share some-small-clip.mov
```

the programs: `gate1_zerocopy`, `gate3_ramp`, `metal_caps`, `gate5_share`, `hap_lane`, `decode_speed` and `vt_check` run from any shell. `gate2_present`, `gate4_coldstart` and `hap_play` open a window, and `sign-test.sh` (the signing and notarization gate) needs the keychain, so those four run inside the login session:

```sh
sh spike/mac/session-run.sh gate2 spike/mac/target/release/gate2_present some-10-bit-hevc.mp4 20
```

`gate6_browse` is driven from windows by `spike\gate6.ps1 -Mac user@host`.

## deploy

nothing ships yet. when it does, every build is signed: windows on the dev machine as "Tridant, Inc.", macos on tec-mba with the owner's personal developer id for now. the signing gate in `docs/phase-0.md` confirms both before any player code is written.
