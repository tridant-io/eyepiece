//! what the window tells the player. the same on every system: the window code turns its own messages into these (adr 0006).

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Space,
    F,
    G,
    Left,
    Right,
    Home,
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug)]
pub enum Event {
    Key { key: Key, shift: bool },
    /// the wheel moved by this many notches. away from the user is positive.
    Wheel { notches: i32 },
    /// the pointer is at this place in the picture area, in physical pixels
    PointerMove { x: f32, y: f32 },
    PointerButton { button: Button, down: bool, x: f32, y: f32 },
    /// the pointer left the window
    PointerLeave,
    /// the picture area changed size, in physical pixels
    Resize { w: u32, h: u32 },
    /// physical pixels for one of the design's pixels changed (the window moved to a display with another scale)
    Scale(f32),
    /// a file was dropped on the window
    Open(String),
    Close,
}
