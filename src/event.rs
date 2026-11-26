//! what the window tells the player. the same on every system: the window code turns its own messages into these (adr 0006).

/// the keys the player listens for, by the letter or name on them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Key {
    Space,
    Enter,
    Escape,
    Backspace,
    Delete,
    Tab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    F5,
    /// the keyboard's own back and forward keys, which the mouse's side buttons press too
    BrowserBack,
    BrowserForward,
    /// any key the player has no use for: it is still a key pressed, which a character typed may follow
    Other,
    Digit0,
    Digit1,
    Digit2,
    Minus,
    Equals,
    BracketLeft,
    BracketRight,
    Comma,
    Slash,
    A,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    V,
    W,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug)]
pub enum Event {
    Key { key: Key, mods: Mods },
    /// a character was typed, for the few fields that take text
    Char(char),
    /// the wheel moved by this many notches. away from the user is positive.
    Wheel { notches: i32, mods: Mods },
    /// the pointer is at this place in the picture area, in physical pixels
    PointerMove { x: f32, y: f32 },
    /// `mods`: the keys held with the press
    PointerButton { button: Button, down: bool, x: f32, y: f32, mods: Mods },
    /// shift or control went down or came up: what is held now
    Held(Mods),
    /// the pointer left the window
    PointerLeave,
    /// the picture area changed size, in physical pixels
    Resize { w: u32, h: u32 },
    /// physical pixels for one of the design's pixels changed (the window moved to a display with another scale)
    Scale(f32),
    /// a file was dropped on the window, or handed over by another launch
    Open(String),
    /// the file at this path, which a tab has open, is a different one now and has stopped changing
    Changed(String),
    /// the file at this path, which a tab has open, is no longer there
    Gone(String),
    /// work done in the background has this to say, for a moment, in the window
    Toast(String),
    /// a screenshot was saved: what to say of it, and the file, which the toast offers to reveal
    Saved(String, std::path::PathBuf),
    /// work done in the background is ready to be looked at
    Wake,
    Close,
}
