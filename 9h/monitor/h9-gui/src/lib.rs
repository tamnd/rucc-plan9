//! The host window of the 9h machine (plan 11). The window shows the screen of the machine, and
//! it gives the keyboard and the mouse of the host to the machine. The back end of macOS on arm64
//! is `AppKit` (`cocoa.rs`). Linux has no back end yet, so a session on Linux is headless.
//!
//! The monitor gives the window a `Host`. The window calls it on the main thread of the
//! process, 60 times a second for the screen and once for each input event.
//!
//! The key and mouse rules come from drawterm `gui-cocoa` (MIT licence): a return is a new
//! line, a backspace is `\b`, the arrows and the function keys are the runes of `keyboard.h`,
//! Option-click is button 2, and Command-click is button 3.

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod cocoa;

/// What the window needs from the monitor.
pub trait Host: Send + Sync {
    /// Copy the screen into `pixels` if it changed after the frame `generation`, and set
    /// `generation`. Give the width and the height, or None when the screen did not change or
    /// the guest has no framebuffer yet. Each pixel is a little-endian word of 0x00RRGGBB.
    fn frame(&self, generation: &mut u64, pixels: &mut Vec<u32>) -> Option<(u32, u32)>;
    /// The mouse is at (x, y) in the pixels of the screen, with the buttons of `/dev/mouse`.
    fn mouse(&self, x: u16, y: u16, buttons: u8);
    /// The keys typed, as UTF-8.
    fn keys(&self, utf8: &[u8]);
    /// The person closed the window or typed Command-Q.
    fn quit(&self);
    /// True when the session ended. Then the window closes and `run` returns.
    fn done(&self) -> bool;
}

/// Show a window of `width` by `height` points with the title `title`, and run the event loop
/// of the host until the session ends. Call it on the main thread.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
pub fn run(
    title: &str,
    width: u32,
    height: u32,
    host: std::sync::Arc<dyn Host>,
) -> Result<(), String> {
    cocoa::run(title, width, height, host)
}

/// Linux has no window back end yet.
#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
pub fn run(_: &str, _: u32, _: u32, _: std::sync::Arc<dyn Host>) -> Result<(), String> {
    Err("this host has no window back end yet; use --headless".into())
}

/// True when the host has a window back end.
#[must_use]
pub const fn available() -> bool {
    cfg!(all(target_os = "macos", target_arch = "aarch64"))
}

/// The runes of `keyboard.h` for the keys that have no character.
mod k {
    pub const KF: u32 = 0xF000;
    pub const HOME: u32 = KF | 0x0D;
    pub const UP: u32 = KF | 0x0E;
    pub const PGUP: u32 = KF | 0x0F;
    pub const PRINT: u32 = KF | 0x10;
    pub const LEFT: u32 = KF | 0x11;
    pub const RIGHT: u32 = KF | 0x12;
    pub const DOWN: u32 = 0x80;
    pub const PGDOWN: u32 = KF | 0x13;
    pub const INS: u32 = KF | 0x14;
    pub const END: u32 = KF | 0x18;
    pub const DEL: u32 = 0x7F;
}

/// The rune of `keyboard.h` for the first UTF-16 unit `c` of the characters of a key of
/// `AppKit`, or None when the key has no rune of its own and the characters go as they are.
/// The function keys of `AppKit` are from 0xF700 (`NSUpArrowFunctionKey`).
#[must_use]
pub fn key_rune(c: u16) -> Option<u32> {
    Some(match c {
        0x0D | 0x03 => u32::from(b'\n'),
        0x7F => 0x08,
        0xF700 => k::UP,
        0xF701 => k::DOWN,
        0xF702 => k::LEFT,
        0xF703 => k::RIGHT,
        0xF704..=0xF70F => k::KF + 1 + u32::from(c - 0xF704),
        0xF727 => k::INS,
        0xF728 => k::DEL,
        0xF729 => k::HOME,
        0xF72B => k::END,
        0xF72C => k::PGUP,
        0xF72D => k::PGDOWN,
        0xF72E | 0xF738 => k::PRINT,
        0xF710..=0xF8FF => return Some(0),
        _ => return None,
    })
}

/// The UTF-8 of a rune of `key_rune`, or nothing for 0, a key that Plan 9 does not have.
#[must_use]
pub fn rune_utf8(r: u32) -> Vec<u8> {
    char::from_u32(r)
        .filter(|_| r != 0)
        .map(|c| c.to_string().into_bytes())
        .unwrap_or_default()
}

/// The modifier flags of `AppKit` that the mouse rules use.
pub const OPTION: u64 = 1 << 19;
/// The Command key of `NSEventModifierFlagCommand`.
pub const COMMAND: u64 = 1 << 20;

/// The buttons of `/dev/mouse` for the buttons of `NSEvent.pressedMouseButtons` (bit 0 left,
/// bit 1 right, bit 2 middle) and the modifier flags. With Option, the left button is button 2.
/// With Command, it is button 3.
#[must_use]
pub fn buttons(pressed: u64, flags: u64) -> u8 {
    let mut b = 0u8;
    if pressed & 1 != 0 {
        b |= if flags & OPTION != 0 {
            2
        } else if flags & COMMAND != 0 {
            4
        } else {
            1
        };
    }
    if pressed & 2 != 0 {
        b |= 4;
    }
    if pressed & 4 != 0 {
        b |= 2;
    }
    b
}

/// The point of the screen of `width` by `height` pixels for the point (x, y) of a view of
/// `vw` by `vh` points, with y up as in `AppKit`.
#[must_use]
#[allow(clippy::cast_sign_loss)] // The value is clamped to 0 and more.
pub fn screen_point(x: f64, y: f64, vw: f64, vh: f64, width: u32, height: u32) -> (u16, u16) {
    let f = |v: f64, n: u32| {
        let max = f64::from(n.saturating_sub(1));
        (v.clamp(0.0, max)) as u16
    };
    if vw <= 0.0 || vh <= 0.0 {
        return (0, 0);
    }
    (
        f(x / vw * f64::from(width), width),
        f((vh - y) / vh * f64::from(height), height),
    )
}

/// The lines of scroll that a trackpad gives in small steps. Each full line is one click of
/// button 4 (up) or button 5 (down).
#[derive(Debug, Default)]
pub struct Scroll {
    sum: f64,
}

/// The points of trackpad scroll for one line, as plan 11.5 says.
pub const LINE: f64 = 40.0;

impl Scroll {
    /// Add the scroll `dy` (up is positive). `precise` is true for a trackpad, whose steps are
    /// in points. Give the button of each click: 8 for up and 16 for down.
    pub fn add(&mut self, dy: f64, precise: bool) -> Vec<u8> {
        if !precise {
            self.sum = 0.0;
            return match dy {
                d if d > 0.0 => vec![8],
                d if d < 0.0 => vec![16],
                _ => vec![],
            };
        }
        self.sum += dy;
        let mut out = vec![];
        while self.sum >= LINE {
            self.sum -= LINE;
            out.push(8);
        }
        while self.sum <= -LINE {
            self.sum += LINE;
            out.push(16);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::{COMMAND, OPTION, Scroll, buttons, key_rune, rune_utf8, screen_point};

    #[test]
    fn the_keys_of_the_host_give_the_runes_of_plan_9() {
        assert_eq!(key_rune(0x0D), Some(0x0A));
        assert_eq!(key_rune(0x7F), Some(0x08));
        assert_eq!(key_rune(0xF700), Some(0xF00E));
        assert_eq!(key_rune(0xF701), Some(0x80));
        assert_eq!(key_rune(0xF704), Some(0xF001));
        assert_eq!(key_rune(0xF70F), Some(0xF00C));
        assert_eq!(key_rune(u16::from(b'a')), None);
        assert_eq!(rune_utf8(0xF00E), "\u{F00E}".as_bytes());
        assert!(rune_utf8(0).is_empty());
        assert_eq!(key_rune(0xF730).map(rune_utf8), Some(vec![]));
    }

    #[test]
    fn the_modifiers_change_the_left_button() {
        assert_eq!(buttons(0, 0), 0);
        assert_eq!(buttons(1, 0), 1);
        assert_eq!(buttons(1, OPTION), 2);
        assert_eq!(buttons(1, COMMAND), 4);
        assert_eq!(buttons(2, 0), 4);
        assert_eq!(buttons(4, 0), 2);
        assert_eq!(buttons(7, 0), 7);
    }

    #[test]
    fn a_point_of_the_view_is_a_point_of_the_screen() {
        assert_eq!(screen_point(0.0, 768.0, 1024.0, 768.0, 1024, 768), (0, 0));
        assert_eq!(
            screen_point(1024.0, 0.0, 1024.0, 768.0, 1024, 768),
            (1023, 767)
        );
        assert_eq!(
            screen_point(256.0, 192.0, 512.0, 384.0, 1024, 768),
            (512, 384)
        );
        assert_eq!(screen_point(-5.0, 900.0, 1024.0, 768.0, 1024, 768), (0, 0));
        assert_eq!(screen_point(1.0, 1.0, 0.0, 0.0, 1024, 768), (0, 0));
    }

    #[test]
    fn a_trackpad_scrolls_one_click_for_each_line() {
        let mut s = Scroll::default();
        assert!(s.add(30.0, true).is_empty());
        assert_eq!(s.add(30.0, true), vec![8]);
        assert_eq!(s.add(-100.0, true), vec![16, 16]);
        assert_eq!(s.add(-1.0, false), vec![16]);
        assert_eq!(s.add(3.0, false), vec![8]);
    }
}
