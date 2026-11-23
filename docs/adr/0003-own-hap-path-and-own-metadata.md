# Our own HAP path, and our own read of metadata

FFmpeg demuxes everything and decodes most codecs, but two things are deliberately not left to it.

**HAP is decoded by our code.** FFmpeg's HAP decoder expands the texture blocks to RGBA on the CPU and cannot decode HAP R (BC7) or HAP HDR (BC6H) at all. We take the packets from FFmpeg's demuxer, parse the HAP sections, decompress Snappy chunks in parallel and upload the BC blocks to the GPU as stored: four to eight times fewer bytes per frame, and every variant covered. This is the main reason the player is not a skin on an existing one.

**Metadata facts come from our own read, not FFmpeg's summary fields.** Its frame count ignores edit lists, its headline frame rate is documented as a guess, it misreads one form of QuickTime timecode track, and its ProRes range and NotchLC color are constants in the decoder, not facts from the file. The owner requires exact values or an explicit "the file does not say".

## Consequences

- A HAP parser and a YCoCg shader to maintain, tested against the reference decoder's block output.
- If FFmpeg's demuxer turns out not to hand over HAP R and HAP HDR packets, HAP also needs its own MOV sample reader. Phase 0 checks this.
- Small parsers for the timecode track and color tags, and tests that use clips with a known frame count instead of ffprobe.
