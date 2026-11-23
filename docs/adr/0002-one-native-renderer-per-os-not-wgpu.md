# One small native renderer per OS, not wgpu

The stack research recommended wgpu, on condition of a cold-start test: one graphics layer for three systems. The engineering review then showed that wgpu offers only an unsafe, per-backend way to take a hardware decoder's surface and no way to learn when a frame reached the screen, so the two hardest parts would be per-OS code either way, and its cold start was unmeasured. The two sources disagreed; we sided with the review and the owner agreed (2026-11-23). Windows renders with Direct3D 11, macOS with Metal, Linux is chosen at its phase (OpenGL or Vulkan).

## Consequences

- Three renderers to keep in step, each one textured quad plus the controls' triangles, and shaders written per API.
- Present timing comes from the OS directly: DXGI frame statistics, CADisplayLink, wp_presentation.
- Phase 0's gates test exactly these native paths. A failed gate reopens this decision.
