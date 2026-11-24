# Native window code per OS, not winit

The stack table the owner agreed to had winit for the window and input, with per-system code beside it for the borderless frame, hit-testing and touchpad input. Phase 0 then wrote that per-system window code anyway: Win32 through the `windows` crate, AppKit through `objc2-app-kit`, with the gates' windows, events and presents running on it on both systems. With the hard parts native either way, winit would add a dependency and a second event model for little, so the player's windows are plain Win32 and AppKit code. This was not chosen for speed: winit's start time was never measured.

## Consequences

- Keyboard layouts, text input for the few typed fields, DPI changes and drag and drop are ours to handle on each system.
- One small event type of our own sits between the window code and everything above it, so the player is the same on every system.
- Linux will need a third window backend, which winit would have given us.
