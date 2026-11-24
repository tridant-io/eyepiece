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

#[derive(Debug)]
pub enum Event {
    Key { key: Key, shift: bool },
    /// the wheel moved by this many notches. away from the user is positive.
    Wheel { notches: i32 },
    /// the picture area changed size, in physical pixels
    Resize { w: u32, h: u32 },
    /// a file was dropped on the window
    Open(String),
    Close,
}
