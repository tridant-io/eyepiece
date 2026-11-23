# Rust, not C++

The owner allowed Rust or C++ and left the choice to us (2026-11-23). An engineering review preferred C++ with SDL3 and Dear ImGui because FFmpeg, the graphics APIs and ImGui's ready-made renderers are all C or C++. We chose Rust: the org's native code (owlette's agent and viewer) is already Rust on the same `windows` and `objc2` crates, one cargo build covers three systems, and the code that parses untrusted media files is memory-safe.

## Consequences

- We write the egui painters for Direct3D 11 and Metal ourselves; Dear ImGui would have shipped them.
- FFmpeg is reached through raw FFI, which needs libclang at build time (or checked-in bindings for the pinned version).
- Phase 0 can still overturn this. If the gates fail in Rust for a reason C++ would not share, the fallback is C++ with SDL3, Dear ImGui and the same renderers.
