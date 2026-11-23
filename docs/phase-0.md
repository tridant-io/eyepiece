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
| 1 | does a hardware-decoded 10-bit hevc frame reach the screen with no cpu copy? | the renderer samples the decoder's own texture on the same device. no transfer to system memory anywhere in the path. the picture matches a software decode of the same frame within 1 code value at 10-bit. | **pass** 2026-11-23 | not run |
| 2 | can we learn when a frame reached the screen? | the os returns present counts and vsync timestamps that advance once per refresh, and the measured refresh rate is stable to 0.01 Hz. a 59.94 clip on a 60 Hz display shows the expected repeat cadence in those numbers, confirmed from outside the app. | not run | not run |
| 3 | does 10-bit survive to the swapchain? | a 1024-step ramp drawn into a 10-bit back buffer reads back with every code value exact. | **pass** 2026-11-23 | not run |
| 4 | how long is a cold start, stage by stage? | measured, not judged: process start, window visible, gpu ready, file open, first frame decoded, first present. twenty warm runs (p50, p95) and one run after a reboot. target for warm: 250 ms on a local hap or prores file. | not run | not run |
| 5 | can a render overwrite a file we have open? | with our handle open, another process can overwrite in place, rename a new file over it, and delete and recreate it. we notice each change from size and modified time. | **pass** 2026-11-23 | not run |
| 6 | does browsing a share open nothing? | on a scratch share that logs opens, listing a folder and reading names, sizes and dates logs zero opens. recorded alongside: which of list, stat and open changes ctime. never run against the owner's nas. | not run | not run |

## measured alongside

| what | why it matters | windows | macos |
|---|---|---|---|
| does ffmpeg's demuxer hand over hap r and hap hdr packets? | if not, hap needs its own mov sample reader (adr 0003) | **yes** 2026-11-23 | n/a |
| hap q at 8K60, including a one-chunk file: disk MB/s, snappy ms per frame by thread count, upload ms | the hap lane's real ceiling | not run | not run |
| notchlc: time for one lone frame step at 4K | its decoder is frame-threaded only, so a single frame decodes on one core | not run | not run |
| prores 4444 xq at 4K60 in software | no hardware prores decode on windows | not run | n/a |
| can a windows touchpad be told from a mouse wheel? | two-finger pan on windows depends on it | not run | n/a |
| sign a throwaway binary here; notarize one on tec-mba | every build is to be signed from the first one | not run | not run |

## log

### 2026-11-23, dev pc (windows 11, rtx 2080 ti, ffmpeg n9.0.2-62 lgpl shared)

**gate 1, zero copy: pass.** `gate1_zerocopy hevc10_4k_5994.mp4 8`
- the decoder ran as direct3d 11 video on our own device. its texture is a 3840 x 2176 array of 17 slices in P010, bound for both decoding and shader reads.
- the renderer drew from two views onto that texture. nothing on the display path was copied to system memory.
- hardware planes against a software decode of the same 8 frames: 0 samples differ.
- drawn pixels against the same math on the cpu: 36,326,400 values compared, worst difference 1 code value at 10-bit, none off by more than 1.
- the mp4 was opened without ffmpeg's stream probing, and that was enough.

**gate 3, 10-bit to the swapchain: pass.** `gate3_ramp`
- a 1024-step ramp in an R10G10B10A2 flip-model back buffer read back with 0 wrong pixels. this proves the path to the swapchain, not the cable or the panel.

**gate 5, shared open: pass.** `gate5_share tiny_hap.mov`

| file held open by | overwrite in place | rename over | delete and recreate |
|---|---|---|---|
| ours: read + write + delete sharing | allowed, noticed | allowed, noticed | allowed, noticed |
| read + write sharing only | allowed, noticed | access denied | sharing violation |
| ffmpeg's own open | allowed, noticed | access denied | sharing violation |

- so ffmpeg's own open blocks the careful kind of renderer. the player must open files itself, with delete sharing, and hand ffmpeg a custom i/o context.
- after a rename-over or a recreate, our old handle keeps reading the old file. a reload has to reopen by path.

**hap packets under tags ffmpeg cannot decode: delivered.** `hap_demux`
- a 4K hap q clip, the same clip written with 1 chunk, and copies retagged Hap7 and HapH: 180 of 180 packets each, every one starting with a valid hap section header. ffmpeg reports the retagged streams as codec "none" but still gives size, frame count and every packet from the header alone.
- so hap r and hap hdr need no mov reader of our own. caveat: the retagged clips carry hap q payloads. a real hap r file has not been tried, because nothing here can encode one.
