# Trim and export write new files: copies in the player, encodes in a separate process

The plan (section 07) put conversion under "no, a different tool", and adr 0005 said the player only decodes. The owner asked for trim, removing sound and export (2026-11-26), and approved the plan in `trim and export: the plan` with "go". So eyepiece now writes video files, in two ways:

- a **copy** (trim, remove sound, a change of container) moves the file's packets unchanged, inside the player, with the four ffmpeg libraries it already ships. it adds no size, keeps the shared open of the source (adr 0005's files stay openable by a render), and takes under a second for a 578 MB prores file (0.39 s measured with ffmpeg's program doing the same work).
- an **encode** (a preset, advanced settings, a pasted ffmpeg command) runs ffmpeg's program as a separate process at below-normal priority, so an encode can never slow or crash playback. it needs the program and three more libraries, about 40 MB (estimated), and comes after a legal review of h.264, hevc, aac and prores encoding.

A new file goes beside the source by default, never over an existing one. Replacing the original is offered behind a prompt: the new file is written and checked first, then the original goes to the recycle bin or trash, never deleted, so not on a network volume. This reverses the "never write beside a media file" rule for what the user asks for.

## Consequences

- A copy of a long-gop file (h.264, hevc) cuts only at key frames: it is labeled with the frame it really starts on, and an exact cut needs an encode.
- Every copy writes the timecode of its first frame (a copy keeps the source's start otherwise) and keeps rotation, color tags and the creation time.
- While an encode reads a source, ffmpeg's program holds it without delete sharing: a render cannot replace that file until the encode ends.
- adr 0005's "none are needed, the player only decodes" no longer holds for encodes; its lgpl rule still does.
