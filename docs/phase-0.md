# phase 0: the gates

six pass or fail questions, answered on real hardware before any player code is written. the stack in `docs/adr/` is kept or changed on these answers. the programs live in `spike/` and are thrown away afterwards.

a gate is run on windows first (this dev machine), then on macos (tec-mba). a result is only written here with the machine, the date and the numbers it printed.

## machines

| name | what | notes |
|---|---|---|
| dev pc | windows 11 pro, 20 logical cores, nvidia rtx 2080 ti (driver 610.88), samsung 990 pro nvme, one display at 3840 × 2160 measured at 59.9964 Hz | a parsec virtual display adapter is also installed. touchdesigner and a browser were open during the runs. |
| tec-mba | macbook air, apple m2, 16 GB, macos 26.6. 12.5 GB free on a volume that is 97% full | not reached yet. ssh as `admin` was refused, and owlette's hoot can read its state but cannot run a command or copy a file there (asked 2026-11-23) |

## the gates

| # | question | passes when | windows | macos |
|---|---|---|---|---|
| 1 | does a hardware-decoded 10-bit hevc frame reach the screen with no cpu copy? | the renderer samples the decoder's own texture on the same device. no transfer to system memory anywhere in the path. the picture matches a software decode of the same frame within 1 code value at 10-bit. | **pass** 2026-11-23 | not run |
| 2 | can we learn when a frame reached the screen? | the os returns present counts and vsync timestamps that advance once per refresh, and the measured refresh rate is stable to 0.01 Hz. a 59.94 clip on a 60 Hz display shows the expected repeat cadence in those numbers, confirmed from outside the app. | **pass** on its own numbers 2026-11-23. not yet confirmed from outside the app | not run |
| 3 | does 10-bit survive to the swapchain? | a 1024-step ramp drawn into a 10-bit back buffer reads back with every code value exact. | **pass** 2026-11-23 | not run |
| 4 | how long is a cold start, stage by stage? | measured, not judged: process start, window visible, gpu ready, file open, first frame decoded, first present. twenty warm runs (p50, p95) and one run after a reboot. target for warm: 250 ms on a local hap or prores file. | **measured** 2026-11-23: 220 to 310 ms warm. target missed. after-reboot run owed | not run |
| 5 | can a render overwrite a file we have open? | with our handle open, another process can overwrite in place, rename a new file over it, and delete and recreate it. we notice each change from size and modified time. | **pass** 2026-11-23 | not run |
| 6 | does browsing a share open nothing? | on a scratch share that logs opens, listing a folder and reading names, sizes and dates logs zero opens. recorded alongside: which of list, stat and open changes ctime. never run against the owner's nas. | not run | not run |

## measured alongside

| what | why it matters | windows | macos |
|---|---|---|---|
| does ffmpeg's demuxer hand over hap r and hap hdr packets? | if not, hap needs its own mov sample reader (adr 0003) | **yes** 2026-11-23, on real hap r and hap hdr files too | n/a |
| hap q at 8K60, including a one-chunk file: disk MB/s, snappy ms per frame by thread count, upload ms | the hap lane's real ceiling | **measured** 2026-11-23: 4K60 solid, 8K60 at the edge | not run |
| notchlc: time for one lone frame step at 4K | its decoder is frame-threaded only, so a single frame decodes on one core | **measured** 2026-11-23: 76 ms | not run |
| prores 4444 xq at 4K60 in software | no hardware prores decode on windows | **measured** 2026-11-23: 30 to 37 fps on worst-case content | n/a |
| can a windows touchpad be told from a mouse wheel? | two-finger pan on windows depends on it | not run: this pc has no touchpad (the hid entry is its logitech receiver). needs another windows machine | n/a |
| sign a throwaway binary here; notarize one on tec-mba | every build is to be signed from the first one | **pass** 2026-11-23 | not run |

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

**gate 2, present timing: pass on its own numbers.** `gate2_present hevc10_4k_5994.mp4 20`
- the os returns present statistics on a flip-model swapchain in a window: 1,195 distinct readings in 1,200 presents.
- measured refresh rate 59.9964 Hz. over one-second windows it ran from 59.9963 to 59.9965 Hz.
- showing every frame for one refresh (4K 10-bit hevc, hardware decode, drawn from the decoder's texture): 1,200 presents in 20.0 s, and 0 refreshes went by without a new frame.
- in a second 20 s run, 1,200 presents were matched by exactly 1,200 refreshes on the os's own count.
- **as first written this gate failed.** its last test kept the clip at its own 59.9401 fps by asking the wall clock, at the moment of each call, which frame was due. a slip of one frame in 17.8 s was predicted; jitter gave 4 to 5 repeats and skips. the test was then changed to pick the frame by counting refreshes. the repeats under that rule (2 in 20 s) are arithmetic the code chose, not a measurement, so they prove nothing by themselves. what was measured is the two lines above: no missed refresh, and presents equal to refreshes.
- not done: confirmation from outside the app. that needs a present-capture tool run as administrator, or a camera.

**gate 4, cold start: measured. the 250 ms target is missed.** `gate4_coldstart <file> 15`. three readings for hap over about an hour. ms since the os created the process, p50 of warm launches:

| reached | hap q 4K, best reading | hap q 4K, other two readings | prores 422 hq 4K, software (one reading, the fast hour) | hevc 10-bit 4K, hardware (one reading, the fast hour) |
|---|---|---|---|---|
| main() entered | 20 | 26 to 31 | 21 | 20 |
| file header read | 23 | 28 to 33 | 23 | 22 |
| window on screen | 46 | 67 to 69 | 48 | 76 |
| first frame decoded | 51 | 73 to 77 | 190 | 253 |
| gpu device ready | 191 | 249 to 266 | 248 | 189 |
| present() returned | 213 | 287 to 296 | 290 | 260 |
| frame visible | 223 | 299 to 312 | 301 | 274 |

- everything except the gpu is ready by about 50 to 75 ms. creating the hardware direct3d 11 device takes 155 to 170 ms in a fresh process by itself (`devtime`), and more when a decoder is busy on the other cores. no flag, feature level or adapter choice changed that. a second device in the same process takes 35 to 40 ms, so it is the driver loading.
- this cost belongs to the driver, not to rust or to our renderer. a c++ player on direct3d would pay the same.
- tried and dropped: painting the first frame with the software rasterizer while the hardware device starts. direct3d serializes device creation, so the software device either waits for the hardware one or, made first, delays everything else. the hap first frame came no earlier than 194 ms that way, and the hardware frame came later.
- not tried: converting the first frame on the cpu with our own code and putting it up with gdi. everything it needs is ready by about 55 ms on the hap lane.
- still owed: one launch straight after a reboot.
- readings an hour apart differed by 30 to 40% on an otherwise idle machine. prores and hevc were read once, in the same hour as the best hap reading; when hap read 300 they would have read higher too. treat every number here as this machine on this day.

**the hap lane.** `hap_speed`, files in the os cache, vsync off. these are cpu-side timings: nothing in this table shows that the gpu kept up or that 60 frames a second reached the display. the playback table under it does.

| clip | on disk | in ram: decompress, one copy, draw | from the file, our own reads | from the file, ffmpeg's reader |
|---|---|---|---|---|
| hap q 4K, real-sized frames (7.9 MB) | 475 MB/s needed | 1.5 to 2.4 ms a frame | 4.0 ms (250 fps) | 5.6 ms (178 fps) |
| hap q 4K, well-compressed, 8 chunks | 40 MB/s | 1.7 to 2.2 ms | 1.9 ms | 1.9 ms |
| hap q 4K, well-compressed, 1 chunk | 40 MB/s | 2.2 to 2.4 ms | 2.4 ms | 2.3 ms |
| hap alpha 4K | 300 MB/s | 2.2 to 4.9 ms | 4.1 ms | 5.1 ms |
| hap q 8K, 16 chunks (31.6 MB a frame) | 1,898 MB/s | 6.2 to 8.3 ms | 15.8 ms (63 fps) | 21.3 ms (47 fps) |
| hap q 8K, 1 chunk | 1,898 MB/s | 7.9 to 8.3 ms | 18.3 ms (55 fps) | 24.1 ms (42 fps) |

- **decompressing straight into the gpu's mapped memory is a trap.** it is fine for frames that do not compress (1.25 ms) and ruinous for frames that do: 39 ms with 8 chunks, 146 ms with 1 chunk, 81 ms for hap alpha. snappy reads back what it has written, and mapped gpu memory is very slow to read. decompress in ordinary ram, then copy once.
- **8K60 hap q does not play through ffmpeg's reader**, even from the os cache: 42 to 47 fps. reading each frame ourselves by the offset and size in ffmpeg's index gives 3.2 GB/s against 1.6 to 1.8.
- **from the file at one frame per refresh, vsync on** (`hap_play`, 20 s each, missed refreshes by the os's count):

  | clip | read and decompress on one thread, present on another | read, decompress and present on three threads |
  |---|---|---|
  | hap q 4K60 (475 MB/s) | 0 missed | 0 missed |
  | hap q 8K60, 16 chunks (1,898 MB/s) | 124 missed, 51 fps | 0 missed in 1,201 frames |
  | hap q 8K60, 1 chunk | 0 missed in 899 frames | 66 missed, 57 fps |

  4K60 is solid, with about 14 ms to spare in every frame. 8K60 on frames that do not compress is at the edge on this machine: each arrangement held one clip and lost the other. per frame it costs about 10 ms to read, 2 to 5 to decompress, 5 to 6 to copy to the gpu and 4 to 11 to draw and present, and the stages fight over memory bandwidth, not disk or gpu. not proven.
- snappy stops scaling at about 4 threads. a one-chunk file costs one core and is still fast enough: 1.2 ms a frame at 4K, 3.9 ms at 8K.
- the first frame drawn by the gpu differs from ffmpeg's own cpu hap decoder by at most 3 to 6 code values at 8-bit, mean 0.2 to 0.5. the plain bc1 and bc3 clips, which have no ycocg step, show the largest difference, so block decoding itself differs between this gpu and ffmpeg. the cause is not verified. a hap pixel readout needs its own defined cpu decode, checked against the reference decoder.

**software decode.** `decode_speed`, all cores, noisy worst-case content:

| clip | straight through | one lone step, frame threads | one lone step, slice threads |
|---|---|---|---|
| prores 422 hq 4K60 | 71 to 89 fps | 213 to 255 ms | 16 ms |
| prores 4444 xq 4K60 | 30 to 37 fps | 441 to 539 ms | 37 ms |
| h.264 4K60, 250-frame groups | 289 to 387 fps | 333 to 474 ms (up to 231 frames decoded) | 3 s |
| hevc 10-bit 4K (software, for reference) | 131 fps | 461 ms (up to 55 frames) | not run |

- prores 4444 xq at 4K60 does not play in real time in software on this machine with this content. 422 hq does, with a margin of 1.2 to 1.5.
- a decoder set up for playback (frame threads) is the wrong one for stepping: it wants a frame per thread before it gives one back. the player needs a second, slice-threaded decoder for steps on intra codecs.
- even then a lone prores step at 4K is 16 to 37 ms of decoding before upload. "one refresh" holds for hap, not for prores, unless the neighbors of the current frame are decoded ahead while paused.

## what the gates changed in the design

| the plan said | the gates say |
|---|---|
| 8K hap q at 60 fps is bounded by the disk | on this machine it is bounded by memory traffic, and it is at the edge even from the os cache. 4K60 is easy. |
| snappy chunks decompress straight into the gpu's upload buffer | decompress in ram, then one copy. the direct way is up to 100 times slower on real content. |
| ffmpeg demuxes; a reader thread feeds it | ffmpeg only supplies the index. we read each frame ourselves by offset and size. needed three times over: 8K60, random access to any frame, and opening files with delete sharing. |
| files are opened shared | confirmed necessary, and ffmpeg's own open cannot do it: it blocks rename-over and delete-and-recreate. |
| cold launch to first frame in 250 ms | 220 to 310 ms for hap, about 300 for prores and 275 for hevc. the hardware device alone is 155 to 250 ms. the second-launch hand-off matters more than planned, because it skips exactly that. |
| the display is the clock for silent clips; audio is the clock otherwise | holds, with one rule added: choose the frame by counting refreshes from the os's statistics, never by reading the wall clock at the call. |
| one frame step is one refresh for hap, prores and notchlc | true for hap. prores needs a slice-threaded stepping decoder and decoded neighbors. notchlc is unmeasured. |
| rust with a native direct3d 11 renderer (adr 0001, 0002) | holds on windows. zero-copy hardware frames, present statistics and 10-bit output all work through the windows crate with no surprises. nothing measured so far argues for c++. macos is untested. |

## what is blocked, and on whom

| what | needs |
|---|---|
| every gate on macos | a shell on tec-mba. hoot has no command or file tools there, so the owner has to authorize this pc's ssh key on the mac (one line in its terminal) and say which user. |
| gate 6 | the owner's ok to install samba inside this pc's wsl ubuntu for a scratch share that logs opens. look-only itself is agreed (adr 0004). |
| gate 4 after a reboot | the owner, when they next reboot (not now, by their word): run one command first thing. |
| gate 2 from outside the app | a present-capture tool run as administrator, or a camera. |
| touchpad against wheel | a windows machine that has a touchpad. this one does not. the design does not wait on it: on windows two fingers scrub and click-drag pans, as the brief has it. |
| signing on macos | the same shell on tec-mba. |

### 2026-11-23, later the same day: the owner's answers, and what they unblocked

the owner said: look-only is fine; render the missing clips from a second touchdesigner instance; sign a throwaway binary; no reboot yet; this pc has no touchpad; reach tec-mba through owlette.

**signing on windows: pass.** owlette's `scripts/sign.ps1` signed a throwaway copy of a gate binary through azure artifact signing (profile `tridantpublic`) and signtool verified it: signer "Tridant, Inc.", rfc 3161 timestamp, status valid. the same script will sign eyepiece builds unchanged.

**real hap r and hap hdr files.** rendered at 4K60 from touchdesigner 2025.33230 (a second instance, closed afterwards), 120 frames each, with a notchlc and a hap q clip.
- ffmpeg reports both as codec "none" (tags Hap7 and HapH) and still delivers 120 of 120 packets, each a valid hap section: bc7 for hap r, bc6h unsigned for hap hdr. the earlier caveat about retagged clips is closed.
- our lane handles them as it does hap q: about 1.5 ms a frame from ram, and 60 frames a second from the file for 12 s with 0 missed refreshes on each.
- not checked: that the picture is right. ffmpeg has no decoder to compare with, so that needs the reference decoder.

**notchlc 4K60.** `decode_speed notchlc_4k60_td.mov`: decoded as yuva444p12le. straight through on all cores, 100 frames a second (1.67 times real time). one lone step costs 201 ms with the playback decoder and 76 ms with a stepping decoder, because notchlc decodes one frame on one core. so a notchlc step is about five refreshes unless the neighbors are decoded ahead.

**tec-mba.** owlette lists it online (apple m2, 16 GB, macos 26.6). asked what it can do there, hoot answered: read system state, start and stop owlette-managed processes, take a screenshot. no command execution, no file transfer. so no macos gate can run through it. the mac also has only 12.5 GB free.

**a side effect to know about.** the scripted touchdesigner setup (embody's bootstrap) treats the enclosing git repo as its project: it appended a block to this repo's `.gitignore` and created a `.gitattributes`. both were undone at once (the first restored from git, the second moved aside into the ignored `spike/media/td-render`). next time, make the throwaway project outside any repo.
