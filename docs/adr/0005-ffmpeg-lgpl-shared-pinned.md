# FFmpeg as LGPL shared libraries, one pinned version

FFmpeg is used only as an LGPL build, shipped as separate shared libraries and never linked statically, with no GPL or non-free components, so the player itself stays free of copyleft. One version is pinned, because the metadata and HAP code depend on behavior that changes between releases.

## Consequences

- No x264, x265 or other GPL encoders or filters; none are needed, the player only decodes.
- FFmpeg's ProRes and NotchLC decoders are unofficial, and software H.264 and HEVC decode carries patent exposure. That is acceptable for internal use and must be settled before any public download.
- The libraries load after the single-instance check, so a second launch never pays for them.
