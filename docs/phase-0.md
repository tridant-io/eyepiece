# phase 0: the gates

six pass or fail questions, answered on real hardware before any player code is written. the stack in `docs/adr/` is kept or changed on these answers. the programs live in `spike/` and are thrown away afterwards.

a gate is run on windows first (this dev machine), then on macos (tec-mba). a result is only written here with the machine, the date and the numbers it printed.

## machines

| name | what | notes |
|---|---|---|
| dev pc | windows 11 pro, nvidia rtx 2080 ti (driver 610.88), one display at 3840 × 2160, reported 59 Hz | a parsec virtual display adapter is also installed |
| tec-mba | macbook air | not reached yet: ssh as `admin` was refused on 2026-11-23 |

## the gates

| # | question | passes when | windows | macos |
|---|---|---|---|---|
| 1 | does a hardware-decoded 10-bit hevc frame reach the screen with no cpu copy? | the renderer samples the decoder's own texture on the same device. no transfer to system memory anywhere in the path. the picture matches a software decode of the same frame within 1 code value at 10-bit. | not run | not run |
| 2 | can we learn when a frame reached the screen? | the os returns present counts and vsync timestamps that advance once per refresh, and the measured refresh rate is stable to 0.01 Hz. a 59.94 clip on a 60 Hz display shows the expected repeat cadence in those numbers, confirmed from outside the app. | not run | not run |
| 3 | does 10-bit survive to the swapchain? | a 1024-step ramp drawn into a 10-bit back buffer reads back with every code value exact. | not run | not run |
| 4 | how long is a cold start, stage by stage? | measured, not judged: process start, window visible, gpu ready, file open, first frame decoded, first present. twenty warm runs (p50, p95) and one run after a reboot. target for warm: 250 ms on a local hap or prores file. | not run | not run |
| 5 | can a render overwrite a file we have open? | with our handle open, another process can overwrite in place, rename a new file over it, and delete and recreate it. we notice each change from size and modified time. | not run | not run |
| 6 | does browsing a share open nothing? | on a scratch share that logs opens, listing a folder and reading names, sizes and dates logs zero opens. recorded alongside: which of list, stat and open changes ctime. never run against the owner's nas. | not run | not run |

## measured alongside

| what | why it matters | windows | macos |
|---|---|---|---|
| does ffmpeg's demuxer hand over hap r and hap hdr packets? | if not, hap needs its own mov sample reader (adr 0003) | not run | n/a |
| hap q at 8K60, including a one-chunk file: disk MB/s, snappy ms per frame by thread count, upload ms | the hap lane's real ceiling | not run | not run |
| notchlc: time for one lone frame step at 4K | its decoder is frame-threaded only, so a single frame decodes on one core | not run | not run |
| prores 4444 xq at 4K60 in software | no hardware prores decode on windows | not run | n/a |
| can a windows touchpad be told from a mouse wheel? | two-finger pan on windows depends on it | not run | n/a |
| sign a throwaway binary here; notarize one on tec-mba | every build is to be signed from the first one | not run | not run |

## log

nothing run yet.
