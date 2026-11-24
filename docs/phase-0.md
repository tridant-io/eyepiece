# phase 0: the gates

six pass or fail questions, answered on real hardware before any player code is written. the stack in `docs/adr/` is kept or changed on these answers. the programs live in `spike/` and are thrown away afterwards.

a gate is run on windows first (this dev machine), then on macos (tec-mba). a result is only written here with the machine, the date and the numbers it printed.

## machines

| name | what | notes |
|---|---|---|
| dev pc | windows 11 pro, 20 logical cores, nvidia rtx 2080 ti (driver 610.88), samsung 990 pro nvme, one display at 3840 × 2160 measured at 59.9964 Hz | a parsec virtual display adapter is also installed. touchdesigner and a browser were open during the runs. |
| tec-mba | macbook air, apple m2, 16 GB, macos 26.6, built-in display 2880 × 1864. rust 1.98.1 and apple's command line tools (no full xcode); no ffmpeg installed. 18 GB free on a volume that is 96% full | reached over ssh 2026-11-23. it is also a ci runner for another project and may be in use over parsec |

## the gates

| # | question | passes when | windows | macos |
|---|---|---|---|---|
| 1 | does a hardware-decoded 10-bit hevc frame reach the screen with no cpu copy? | the renderer samples the decoder's own texture on the same device. no transfer to system memory anywhere in the path. the picture matches a software decode of the same frame within 1 code value at 10-bit. | **pass** 2026-11-23 | **pass** 2026-11-23 |
| 2 | can we learn when a frame reached the screen? | the os returns present counts and vsync timestamps that advance once per refresh, and the measured refresh rate is stable to 0.01 Hz. a 59.94 clip on a 60 Hz display shows the expected repeat cadence in those numbers, confirmed from outside the app. | **pass** on its own numbers 2026-11-23. not yet confirmed from outside the app | the times come back and the rate is steady, but **not passed**: run only while the mac was busy, and frames were dropped. quiet rerun owed |
| 3 | does 10-bit survive to the swapchain? | a 1024-step ramp drawn into a 10-bit back buffer reads back with every code value exact. | **pass** 2026-11-23 | **pass** 2026-11-23 |
| 4 | how long is a cold start, stage by stage? | measured, not judged: process start, window visible, gpu ready, file open, first frame decoded, first present. twenty warm runs (p50, p95) and one run after a reboot. target for warm: 250 ms on a local hap or prores file. | **measured** 2026-11-23: 220 to 310 ms warm. target missed. after-reboot run owed | **measured** 2026-11-23 while the mac was busy: about 360 ms. quiet rerun owed |
| 5 | can a render overwrite a file we have open? | with our handle open, another process can overwrite in place, rename a new file over it, and delete and recreate it. we notice each change from size and modified time. | **pass** 2026-11-23 | **pass** 2026-11-23 |
| 6 | does browsing a share open nothing? | on a scratch share that logs opens, listing a folder and reading names, sizes and dates logs zero opens. recorded alongside: which of list, stat and open changes ctime. never run against the owner's nas. | **pass** 2026-11-23, against samba 4.23.8 | **pass** 2026-11-23 |

## measured alongside

| what | why it matters | windows | macos |
|---|---|---|---|
| does ffmpeg's demuxer hand over hap r and hap hdr packets? | if not, hap needs its own mov sample reader (adr 0003) | **yes** 2026-11-23, on real hap r and hap hdr files too | n/a |
| hap q at 8K60, including a one-chunk file: disk MB/s, snappy ms per frame by thread count, upload ms | the hap lane's real ceiling | **measured** 2026-11-23: 4K60 solid, 8K60 at the edge | **measured** 2026-11-23 at 4K while the mac was busy: 2.1 to 3.5 ms a frame from ram. 8K not run |
| notchlc: time for one lone frame step at 4K | its decoder is frame-threaded only, so a single frame decodes on one core | **measured** 2026-11-23: 76 ms | **measured** 2026-11-23 while busy: 65 ms |
| prores 4444 xq at 4K60 in software | no hardware prores decode on windows | **measured** 2026-11-23: 30 to 37 fps on worst-case content | **measured** 2026-11-23 while busy: 14 fps in software, 67 fps through videotoolbox |
| can a windows touchpad be told from a mouse wheel? | two-finger pan on windows depends on it | not run: this pc has no touchpad (the hid entry is its logitech receiver). needs another windows machine | n/a |
| does the gpu take hap's block formats (bc1, bc3, bc4, bc5, bc6h, bc7) as stored? | hap's lane uploads blocks undecoded; a gpu without them would need a cpu decode | yes, by construction of direct3d 11 | **yes** 2026-11-23, apple m2, at 8K |
| sign a throwaway binary here; notarize one on tec-mba | every build is to be signed from the first one | **pass** 2026-11-23 | **pass** 2026-11-23, signed and notarized with ffmpeg inside |

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
| on a network folder, use the folder listing and nothing else (adr 0004) | holds, and the line is exact. the listing sends one open, of the folder, and no read. asking for one file's details by path (`std::fs::metadata`) opened a file even straight after a listing. so names, sizes and dates come from the listing's own entries, never from a call per file. |
| prores decodes in software everywhere | on the mac prores goes through videotoolbox: three to five times the throughput of software on the m2 and a lone step in 6 to 11 ms. software prores stays the windows lane and the mac's fallback. |
| the first present shows the first frame | on the mac the first two presents after a window opens never reach the display. the player keeps presenting until the system reports one shown, and only then counts the picture as up. |
| a network folder is listed once, with sizes and dates | true on windows. on the mac, names come in one listing but sizes and dates cost a request per file: names first, sizes and dates filled in for what is on screen. |
| a changed file is seen through the open handle | not on the mac: after a rename-over or a recreate the handle keeps reading the old file. the player watches the path (size, time, which file the name points at) and reopens. on windows the same watch works. |
| notchlc at 4K60 plays in software | 100 fps on the windows pc. 32 fps on the m2 while it was busy, with no hardware path to fall back on: in doubt there until rerun on a quiet machine. |
| rust with a native direct3d 11 renderer (adr 0001, 0002) | holds on windows: zero-copy hardware frames, present statistics and 10-bit output all work through the windows crate with no surprises. the metal half holds as far as tested: device, shaders, drawing, a 10-bit layer and videotoolbox frames sampled in place, through the objc2 crates. present timing on the mac is untested. nothing measured argues for c++. |

## what is blocked, and on whom

| what | needs |
|---|---|
| gates 2 and 4 and hap playback on macos, again | a quiet mac. all three ran while it was busy with other work (load average 16 to 28 on 8 cores), so the dropped frames and the times are not yet the mac's own. the reruns need only that its other work stops. the speeds measured there carry the same caveat. |
| gate 4 after a reboot | the owner, when they next reboot (not now, by their word): run one command first thing. |
| gate 2 from outside the app | a present-capture tool run as administrator, or a camera. |
| touchpad against wheel | a windows machine that has a touchpad. this one does not. the design does not wait on it: on windows two fingers scrub and click-drag pans, as the brief has it. |

### 2026-11-23, later the same day: the owner's answers, and what they unblocked

the owner said: look-only is fine; render the missing clips from a second touchdesigner instance; sign a throwaway binary; no reboot yet; this pc has no touchpad; reach tec-mba through owlette.

**signing on windows: pass.** owlette's `scripts/sign.ps1` signed a throwaway copy of a gate binary through azure artifact signing (profile `tridantpublic`) and signtool verified it: signer "Tridant, Inc.", rfc 3161 timestamp, status valid. the same script will sign eyepiece builds unchanged.

**real hap r and hap hdr files.** rendered at 4K60 from touchdesigner 2025.33230 (a second instance, closed afterwards), 120 frames each, with a notchlc and a hap q clip.
- ffmpeg reports both as codec "none" (tags Hap7 and HapH) and still delivers 120 of 120 packets, each a valid hap section: bc7 for hap r, bc6h unsigned for hap hdr. the earlier caveat about retagged clips is closed.
- our lane handles them as it does hap q: about 1.5 ms a frame from ram, and 60 frames a second from the file for 12 s with 0 missed refreshes on each.
- not checked: that the picture is right. ffmpeg has no decoder to compare with, so that needs the reference decoder.

**notchlc 4K60.** `decode_speed notchlc_4k60_td.mov`: decoded as yuva444p12le. straight through on all cores, 100 frames a second (1.67 times real time). one lone step costs 201 ms with the playback decoder and 76 ms with a stepping decoder, because notchlc decodes one frame on one core. so a notchlc step is about five refreshes unless the neighbors are decoded ahead.

**tec-mba.** owlette lists it online (apple m2, 16 GB, macos 26.6). asked what it can do there from this pc, hoot answered: read system state, start and stop owlette-managed processes, take a screenshot. no command execution, no file transfer. the mac also has only 12.5 GB free. (what happened when the owner said to go through owlette anyway is in the next entry.)

**a side effect to know about.** the scripted touchdesigner setup (embody's bootstrap) treats the enclosing git repo as its project: it appended a block to this repo's `.gitignore` and created a `.gitattributes`. both were undone at once (the first restored from git, the second moved aside into the ignored `spike/media/td-render`). next time, make the throwaway project outside any repo.

### 2026-11-23, evening: gate 6, and the mac through owlette

**gate 6 on windows: pass.** `spike\gate6.ps1` brings up a scratch samba share (4.23.8) in its own small wsl distro, makes one folder of 20 files per mode that the windows client (11 pro 23H2) has never seen, runs `gate6_browse` on each, and counts what the server logged. two counts per run: the smb2 requests the server received, by name, and the files its audit module saw a client ask to open.

| mode | open requests | list requests | read requests | files opened | file times changed |
|---|---|---|---|---|---|
| open (the control: open each file, read a byte) | 22 | 2 | 20 | 20 | access time |
| list (the folder listing alone) | 1 | 2 | 0 | 0 | none |
| stat (`std::fs::metadata` per path, after listing) | 2 | 2 | 0 | 1 | none |
| attrs (`GetFileAttributesExW` per path, after listing) | 1 | 2 | 0 | 0 | none |
| list again, under a minute later | 1 | 0 | 0 | 0 | none |

- a listing costs one open, and it is the folder (list and read-attributes access). no file is opened and nothing is read. five runs agreed, the last on a writable share.
- stat opened one file of the 20 (attributes only, access 0x100080); windows answered for the other 19 from what the listing had just told it. attrs opened none. both lean on windows' short-lived copy of the listing, so neither is a rule to build on. the listing's own entries already carry size and dates.
- the second listing never asked the server to list: windows answered from its own copy. samba grants the client a lease on the folder and i take that to be why; not checked. the lease is meant to be withdrawn when the folder changes; not tested either.
- the audit log also shows one `openat` per file while the server answers a listing. that is samba reading each entry's details itself, not a request from the client: the request log has no open for those files, and no file time moves.
- the change time moved in no mode, including the control, on a read-only share and again on a writable one. reading changed the access time only. so this does not reproduce the owner's nas resetting change times on bulk reads; why the nas does that is not known.
- limits: one server, one client, 20 small files. whether a file is opened is decided by the client, and this is the client the player runs on. macos has its own smb client and is not measured.

the share is not the wsl ubuntu the owner agreed to: that distro does not start (its disk file is gone). it runs in a separate throwaway distro, `eyepiece-gate6` (alpine 3.24.4, 76 MB under `%LOCALAPPDATA%\eyepiece-gate6-wsl`), stopped when the script ends. `wsl --unregister eyepiece-gate6` removes it and its disk; left for the owner to say. the script's first-run branch (download and import) was done by hand with the same commands, not through the script.

**tec-mba through owlette: refused, and left refused.** the owner said to put this pc's ssh key on the mac through owlette rather than at the mac's keyboard. that takes a run-once process added to the mac's owlette list. `add_process` answered: "this call changes what the machine launches, so it is tier 3, and this machine's grant is tier 2." nothing was added and nothing ran on the mac. the grant is the owner's setting (owlette admin panel, hoot, this pc's "how far"), so it was not worked around. the owner can add that process themselves in the dashboard; starting one that exists is within this pc's grant. that route is untested on macos.

### 2026-11-23, night: correction. tec-mba was reachable all along

ssh works as user `rosco` with this pc's key, as other projects on this pc have used it for months. the "refused" in the entries above was my wrong user name (`admin`), and i had not looked at those projects' notes. the owlette detour was never needed: its refusal stands as a fact about this pc's grant, not as a block, and nothing was added to the mac. through owlette this pc can read the mac's state and manage its listed processes but has no shell there, so builds and gate runs go over ssh.

### 2026-11-23, night: the mac, first results

tec-mba, apple m2, macos 26.6. the machine was in use throughout (load average 16 on 8 cores: spotlight indexing, a browser, a call app, another session's work), so nothing here is a timing.

**ffmpeg on the mac.** `spike/mac/setup.sh` builds the release/9.0 branch (9.0.2, the same library versions as the windows build): lgpl, shared, videotoolbox, 18 MB installed. the gate programs for the mac are their own small package, `spike/mac`, run with `spike/mac/gate.sh`.

**gate 5 on macos: pass.** nothing is ever refused here, so the gate is about noticing.

| file held open by | writer | allowed | noticed from the path by | our open handle then reads |
|---|---|---|---|---|
| a plain read-only open | overwrite in place | yes | size, time | the new file |
| a plain read-only open | rename over | yes | size, time, identity | the old file |
| a plain read-only open | delete and recreate | yes | size, time, identity | the old file |
| ffmpeg's own open | all three | yes | the same | not read |

- after a rename-over or a recreate our handle goes on reading the old file, silently. so the player watches the path (size, time and which file the name points at) and reopens; it never trusts the handle to show a new render.
- ffmpeg's own open blocks nothing on the mac. reading frames ourselves is still the plan, for the other two reasons found on windows.

**gate 6 on macos: pass.** `spike\gate6.ps1 -Mac user@host`: the same scratch share, reached from the mac through an ssh tunnel and mounted hidden from finder.

| mode | open requests | list requests | read requests | files opened | file times changed |
|---|---|---|---|---|---|
| idle (nothing run, 20 s) | 2 | 0 | 0 | 0 | none |
| open (the control) | 43 | 23 | 21 | 20 | access time |
| names (the listing alone) | 3 | 3 | 0 | 0 | none |
| bulk (`getattrlistbulk`: names, sizes, dates) | 23 | 23 | 0 | 0 | none |
| stat (rust's walk: one stat per file) | 23 | 23 | 0 | 0 | none |
| bulk again | 22 | 23 | 0 | 0 | none |

- no file is opened by any way of listing, and nothing is read.
- but sizes and dates are not free on the mac. names cost one listing for the whole folder. asking for sizes and dates, by either call, made the mac ask the server about each file by name: one open of the folder and one single-name listing per file (the server's log shows 21 opens of the folder for 20 files). windows gets sizes and dates inside the one listing.
- so on a mac, a network folder of 5,000 frames would be 5,000 round trips before sizes show (extrapolated from 20 files; not run at that size). the browser shows names at once and fills sizes and dates in afterwards, for what is on screen first.
- a repeated listing was asked again in full; the mac kept no copy that we could see.
- left alone for 20 s the mac made two requests of its own, neither on a file.
- the windows run now has the same idle row: 0 requests.

**signing on macos: pass, for an app that carries ffmpeg.** `spike/mac/sign-test.sh`, run as a job in the mac's login session (keychains are locked over ssh). it wraps a gate program and the four ffmpeg libraries in a throwaway .app, rewrites the library references to stay inside the bundle, signs each library and then the app with the hardened runtime and a timestamp, runs it (the libraries load), has apple notarize it (accepted), staples the ticket, and asks gatekeeper: "accepted, source=Notarized Developer ID".
- what this proves: the identity, the notary credentials and the ffmpeg-inside-the-bundle shape all work on this machine.
- what it does not: the real app's entitlements, a build for both processor kinds, or a disk image or installer around it.
- the signer is the mac's only identity, a personal developer id. windows signs as "Tridant, Inc.". which name the mac build carries is the owner's to say.

**left on the mac** (sizes at the end of that night, after the entries below). `~/src/eyepiece` (sources, the ffmpeg install, build output and the one-off job files, 855 MB), `/private/tmp/eyepiece-ffmpeg-src` (188 MB), `/private/tmp/eyepiece-media` (one clip, 50 MB), `/private/tmp/eyepiece-signtest-*` (25 MB), `/private/tmp/eyepiece-g6` and `/private/tmp/eyepiece-tiny_hap.mov` (under 1 MB). about 1.1 GB in all; the mac has 16 GB free. no job and no mount is left behind. removing any of it waits for the owner's word.

### 2026-11-23, night: the mac's graphics gates that need no window

same machine and the same caveat: it was in use, so nothing here is a timing. each of these draws into a texture or an unshown layer and puts nothing on the screen.

**the bindings.** the mac programs reach metal through the `objc2` crates (objc2 0.6; objc2-metal, objc2-quartz-core, objc2-foundation, objc2-core-foundation and objc2-io-surface 0.3). the four corevideo calls needed are declared by hand. every program below compiled and ran on its first build. this is the spike's choice; for the player it is the proposal, not yet agreed.

**hap's block formats: yes.** `metal_caps`: the m2 reports block-compressed textures as supported and makes a 7680 × 4320 texture in each of bc1, bc3, bc4, bc5, bc6h and bc7. so the hap lane can upload blocks as stored on this mac, as on windows. (not every metal device has these formats; the player has to ask and fall back.)

**gate 3 on macos: pass.** `gate3_ramp`: a 1024-step ramp drawn into the 10-bit drawable of a metal layer (pixel format bgr10a2) reads back exact: 0 wrong of 49,152 values.

**gate 1 on macos: pass, into a texture.** `gate1_zerocopy hevc10_4k_5994.mp4`, 8 frames at 3840 × 2160:
- videotoolbox hands ffmpeg its own pixel buffer (format 'x420', 10-bit, two planes). two metal textures are made directly on that buffer's surface, one per plane, and the draw samples them. both textures report the decoder's own surface as their memory.
- the decoder's planes, read back through the gpu, against a software decode: 0 samples differ.
- the drawn picture against the same math on the cpu: 49,766,400 values, worst difference 1 code value, none over 1.
- not shown here: the same path presenting to a window. that is gate 2's job.

### 2026-11-23, late night: gates 2 and 4 on the mac, while it was busy

nobody had touched the mac for over half an hour, so the windows disturbed no one. but it was working hard on other things the whole time: load average 16 to 25 on 8 cores. every number below carries that, and both gates are owed a rerun on a quiet machine. each program prints the load beside its result.

**how they run.** a window cannot open from an ssh session. `spike/mac/session-run.sh <name> <command>` runs a command as a one-off job inside the mac's login session and prints its output; it refuses while the screen is locked. the signing gate runs the same way.

**gate 2 on macos: the mechanism works; not passed.** `gate2_present hevc10_4k_5994.mp4 20`: the clip plays in a 1920 × 1080 window for 20 s, each frame drawn from the decoder's own surface (so gate 1's path is now shown presenting), and each present asks the system when it reached the display.

| | |
|---|---|
| load average during the run | 22.3 before, 18.6 after |
| presents sent | 1,200 |
| came back with a time | 1,194 |
| never reached the display | 6: presents 0 and 1, and 172 to 175 |
| refreshes that went by without a new frame | 9 |
| refresh rate from the times | 59.9999 Hz; 59.9998 Hz in each half |
| frames shown twice on purpose (59.94 on 60) | 2, predicted 1.2 |

- the system does report when each frame reached the display, the times step by exactly one refresh, and the rate is steady well inside 0.01 Hz. that is what the design needs.
- the first two presents after the window opens never reach the display. a 5 s run and every cold-start launch showed the same two. so the player must not treat its first present as shown: it keeps presenting until one comes back with a time.
- presents 172 to 175 were dropped in one stumble mid-run. on a machine this busy that proves nothing either way; it is why the gate is not marked passed.

**gate 4 on macos: measured, under load.** `gate4_coldstart hevc10_4k_5994.mp4 20`: 20 fresh launches after one first launch, load average 20.7 before and 21.3 after. times are ms from the moment the launch was asked for.

| stage | first launch | p50 | p95 |
|---|---|---|---|
| process running | 16 | 25 | 40 |
| gpu device | 45 | 56 | 68 |
| file open | 59 | 58 | 69 |
| window on screen | 200 | 189 | 198 |
| shader compiled | 205 | 191 | 201 |
| first frame decoded (videotoolbox, 4K hevc) | 410 | 313 | 327 |
| first frame on the display | 453 | 363 | 391 |

- the two big steps are the window (about 130 ms after the file is open) and the decoder's first frame (about 120 ms). the test does them one after the other; whether they overlap if started together is not tested.
- the metal device is quick here (about 30 ms), unlike direct3d's 155 to 250 ms on windows.
- the last 50 ms is the two dropped presents and the wait for a refresh.
- this is a bare program, not an app launched from finder, and a hevc file, not the hap or prores the 250 ms target names. the hap lane is not written for the mac yet.

### 2026-11-23, late night: the owner's answers, then the mac's hap lane and decode speeds

the owner said: their personal developer id is fine for the mac build for now; the objc2 crates are fine for the mac renderer (now in adr 0002); copy the clips over. cut-down copies of eight clips went to `/private/tmp/eyepiece-media` (2.4 GB).

the mac was busy throughout, as before: load average 15 to 28 on 8 cores. every number here is "at least this good"; none is the mac's best.

**the hap lane from ram, off screen.** `hap_lane <clip> 30`, 3840 × 2160, 7.9 MB of blocks a frame:

| clip | snappy, 1 thread | snappy, 4 threads | the one copy into the texture | whole lane, drawn at full size | against ffmpeg's decode of frame 0 |
|---|---|---|---|---|---|
| hap q, compressible, 8 chunks | 1.47 ms | 0.48 ms | 0.89 ms | 2.83 ms | worst 4 of 255, mean 0.24 |
| hap alpha, noise, 8 chunks | 6.17 ms | 1.61 ms | 0.78 ms | 3.53 ms | worst 3 of 255, mean 0.24 |
| hap r from touchdesigner, 12 chunks | 0.36 ms | 0.27 ms | 0.77 ms | 2.21 ms | not checked: ffmpeg has no decoder |
| hap hdr from touchdesigner, 12 chunks | 1.16 ms | 0.34 ms | 0.81 ms | 2.12 ms | not checked: ffmpeg has no decoder |

- the lane has room: 2 to 3.5 ms of a 16.7 ms refresh, on a busy machine.
- the picture is right for hap q and hap alpha: the gpu's own block decode and our ycocg shader land within 4 of 255 of ffmpeg's software decode, 0.24 on average. hap r and hap hdr still have no referee on either system.
- the copy is a plain copy into a texture the cpu and gpu share. the windows rule (never decompress into the gpu's mapped memory) was about write-combined memory and has no counterpart measured here.

**hap playback from the file, in a window: not passed while busy.** `hap_play <clip> 10`:

| clip | load average | presented | after the first half second: never shown | refreshes without a new frame |
|---|---|---|---|---|
| hap q, 8 chunks | 24.1 to 22.0 | 596 in 10 s | 1 | 8 |
| hap r, 12 chunks | 22.0 to 19.2 | 591 in 10 s | 0 | 10 |

- the reader was never the hold-up (0.01 ms waiting a frame). the copy took 3.8 to 4.7 ms here against 0.8 ms off screen, and a few refreshes were missed. on a machine this loaded that settles nothing; rerun owed.

**decode speeds.** `decode_speed`, the same program as on windows, now also through videotoolbox on the mac. 4K, frames decoded and not drawn:

| clip | software, all cores | videotoolbox | one lone step, software (slice threads) | one lone step, videotoolbox |
|---|---|---|---|---|
| prores 422 hq, 60 fps | 34 fps | 92 fps | 28 ms | 6 ms (p95 29) |
| prores 4444 xq, 60 fps | 14 fps | 67 fps | 72 ms | 11 ms (p95 43) |
| notchlc, 60 fps | 32 fps | not offered | 65 ms | n/a |
| hevc 10-bit, 59.94 fps, key frame every 60 | 62 fps | 285 fps | 1,253 ms | 79 ms (p95 193) |

- on the mac prores belongs on videotoolbox, not in software: three to five times the throughput, and a lone step in 6 to 11 ms. the plan's software prores path is the fallback there, not the lane.
- 4444 xq at 60 fps through videotoolbox ran at 1.12 times real time on a busy machine. close; rerun owed.
- notchlc has no hardware path and managed 32 fps in software here, half of real time. busy or not, 4K60 notchlc on this mac is in doubt; rerun owed.
- a lone step into long-gop hevc costs up to 52 decoded frames even in hardware: 79 ms, 193 at worst.

**the mac's disk.** it had 16 GB free before the clips and 11 GB after; the clips are 2.4 GB of that, so something else on the mac took about 2.5 GB in the same hour. ours in all: about 3.5 GB (`/private/tmp/eyepiece-media` 2.4 GB, `~/src/eyepiece` 845 MB, `/private/tmp/eyepiece-ffmpeg-src` 188 MB, the rest under 30 MB).
