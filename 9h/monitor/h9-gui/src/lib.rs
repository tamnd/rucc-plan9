//! The host window of the 9h machine (plan 11). The window shows the screen of the machine, and
//! it gives the keyboard and the mouse of the host to the machine. The back end of macOS on arm64
//! is `AppKit` (`cocoa.rs`). On Linux, the back end is Wayland (`wayland.rs`) when
//! `WAYLAND_DISPLAY` is set, and X11 (`x11.rs`) when `DISPLAY` is set. `H9_GUI=x11` or
//! `H9_GUI=wayland` chooses one of the two. With neither, a session on Linux is headless.
//!
//! The monitor gives the window a `Host`. The window calls it on the main thread of the
//! process, 60 times a second for the screen and once for each input event.
//!
//! The key and mouse rules come from drawterm `gui-cocoa` (MIT licence): a return is a new
//! line, a backspace is `\b`, the arrows and the function keys are the runes of `keyboard.h`,
//! Option-click is button 2, and Command-click is button 3.

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod cocoa;
#[cfg(unix)]
pub mod vinput;
#[cfg(unix)]
mod wayland;
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
mod x11;

/// What the window needs from the monitor.
pub trait Host: Send + Sync {
    /// Copy the screen into `pixels` if it changed after the frame `generation`, and set
    /// `generation`. Give the width and the height, or None when the screen did not change or
    /// the guest has no framebuffer yet. Each pixel is a little-endian word of 0x00RRGGBB.
    fn frame(&self, generation: &mut u64, pixels: &mut Vec<u32>) -> Option<(u32, u32)>;
    /// The mouse is at (x, y) in the pixels of the screen, with the buttons of `/dev/mouse`.
    fn mouse(&self, x: u16, y: u16, buttons: u8);
    /// The keys typed, as UTF-8: a key down and a key up for each rune.
    fn keys(&self, utf8: &[u8]);
    /// A key down, `down` true, or a key up of the rune `r` of `keyboard.h`. A host that has no
    /// key up events types the rune on its key down.
    fn key(&self, r: u32, down: bool) {
        if down && !is_modifier(r) {
            self.keys(&rune_utf8(r));
        }
    }
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

/// Show a window on the Wayland compositor of `WAYLAND_DISPLAY` or on the X11 display of
/// `DISPLAY`, as [`run`] does on macOS.
#[cfg(target_os = "linux")]
#[allow(clippy::needless_pass_by_value)] // The same signature as the run of macOS.
pub fn run(
    title: &str,
    width: u32,
    height: u32,
    host: std::sync::Arc<dyn Host>,
) -> Result<(), String> {
    match linux_back_end(&|v| std::env::var(v).ok()) {
        Some(LinuxBackEnd::Wayland) => wayland::run(title, width, height, &*host),
        Some(LinuxBackEnd::X11) => x11::run(title, width, height, &*host),
        None => Err("set WAYLAND_DISPLAY or DISPLAY, or use --headless".into()),
    }
}

/// A window back end of Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxBackEnd {
    /// The Wayland compositor of `WAYLAND_DISPLAY`.
    Wayland,
    /// The X11 display of `DISPLAY`.
    X11,
}

/// The back end of Linux for the environment `env`: the back end of `H9_GUI` when it is `x11`
/// or `wayland` and its variable is set, else Wayland when `WAYLAND_DISPLAY` is set, else X11
/// when `DISPLAY` is set.
#[must_use]
pub fn linux_back_end(env: &dyn Fn(&str) -> Option<String>) -> Option<LinuxBackEnd> {
    let set = |v: &str| env(v).is_some_and(|d| !d.is_empty());
    let (wayland, x11) = (set("WAYLAND_DISPLAY"), set("DISPLAY"));
    match env("H9_GUI").as_deref() {
        Some("x11") if x11 => Some(LinuxBackEnd::X11),
        Some("wayland") if wayland => Some(LinuxBackEnd::Wayland),
        Some("x11" | "wayland") => None,
        _ if wayland => Some(LinuxBackEnd::Wayland),
        _ if x11 => Some(LinuxBackEnd::X11),
        _ => None,
    }
}

/// The other hosts have no window back end.
#[cfg(not(any(target_os = "linux", all(target_os = "macos", target_arch = "aarch64"))))]
pub fn run(_: &str, _: u32, _: u32, _: std::sync::Arc<dyn Host>) -> Result<(), String> {
    Err("this host has no window back end; use --headless".into())
}

/// True when the host has a window back end: on macOS on arm64, and on Linux when
/// [`linux_back_end`] gives one.
#[must_use]
pub fn available() -> bool {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        return true;
    }
    cfg!(target_os = "linux") && linux_back_end(&|v| std::env::var(v).ok()).is_some()
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
    pub const ALT: u32 = KF | 0x15;
    pub const SHIFT: u32 = KF | 0x16;
    pub const CTL: u32 = KF | 0x17;
    pub const DEL: u32 = 0x7F;
    pub const SPEC: u32 = 0xF800;
    pub const CAPS: u32 = SPEC | 0x64;
    pub const NUM: u32 = SPEC | 0x65;
    pub const ALTGR: u32 = SPEC | 0x67;
    pub const MOD4: u32 = SPEC | 0x68;
}

/// True for the runes of the modifier keys, which `kbdfs` uses for its state and which do not
/// type a character.
#[must_use]
pub fn is_modifier(r: u32) -> bool {
    matches!(
        r,
        k::ALT | k::SHIFT | k::CTL | k::CAPS | k::NUM | k::ALTGR | k::MOD4
    )
}

/// The keys that are down in a host window, for the key up events of `#b/kbd` (plan 11.4).
/// For each key, it keeps the rune that the key gave when it went down. The key up is for that
/// rune, so no key up is lost when a modifier changes while the key is down. The codes of the
/// keys are the codes of the host: the `keyCode` of `AppKit` or the keycode of X11.
#[derive(Debug, Default)]
pub struct Held {
    keys: Vec<(u32, u32)>,
    /// The modifier flags of `AppKit` that are down.
    flags: u64,
    /// True from an Option down until the next key, while Option can still be a tap.
    option_alone: bool,
}

/// The modifier flags of `AppKit` and their runes. Option is not in the list: alone it is
/// `Kalt`, and with a key it goes to the input method of macOS.
const FLAG_RUNES: [(u64, u32); 3] = [(SHIFT, k::SHIFT), (CONTROL, k::CTL), (COMMAND, k::MOD4)];

/// A key event: the rune, and true for a key down or false for a key up.
pub type KeyEvent = (u32, bool);

impl Held {
    /// The key `code` went down and gives the rune `r`, or 0 for a key that types nothing of its
    /// own. A key that is down already is a repeat of the host, which gives one more key down of
    /// the first rune.
    pub fn press(&mut self, code: u32, r: u32) -> Vec<KeyEvent> {
        self.option_alone = false;
        if r == 0 {
            return vec![];
        }
        if let Some(&(_, first)) = self.keys.iter().find(|(c, _)| *c == code) {
            return vec![(first, true)];
        }
        self.keys.push((code, r));
        vec![(r, true)]
    }

    /// The key `code` went up.
    pub fn release(&mut self, code: u32) -> Vec<KeyEvent> {
        match self.keys.iter().position(|(c, _)| *c == code) {
            Some(i) => vec![(self.keys.remove(i).1, false)],
            None => vec![],
        }
    }

    /// The modifier flags of `AppKit` are now `flags`. Shift, Control and Command give `Kshift`,
    /// `Kctl` and `Kmod4`. Option gives a `Kalt` down and up when it goes up with no key after
    /// its down, as in drawterm, so that a tap of Option starts a compose of Plan 9.
    pub fn modifiers(&mut self, flags: u64) -> Vec<KeyEvent> {
        let mut v = vec![];
        let changed = flags ^ self.flags;
        for (bit, r) in FLAG_RUNES {
            if changed & bit != 0 {
                v.push((r, flags & bit != 0));
            }
        }
        if changed & OPTION != 0 {
            if flags & OPTION != 0 {
                self.option_alone = true;
            } else if std::mem::take(&mut self.option_alone) {
                v.extend([(k::ALT, true), (k::ALT, false)]);
            }
        }
        if flags & OPTION != 0 && flags & (SHIFT | CONTROL | COMMAND) != 0 {
            self.option_alone = false;
        }
        self.flags = flags & (SHIFT | CONTROL | OPTION | COMMAND);
        v
    }

    /// A button of the mouse went down. Then Option is not a tap: Option-click is button 2.
    pub fn mouse_down(&mut self) {
        self.option_alone = false;
    }

    /// The window lost the keyboard: a key up for each key and modifier that is down.
    pub fn release_all(&mut self) -> Vec<KeyEvent> {
        let mut v: Vec<KeyEvent> = std::mem::take(&mut self.keys)
            .into_iter()
            .map(|(_, r)| (r, false))
            .collect();
        for (bit, r) in FLAG_RUNES {
            if self.flags & bit != 0 {
                v.push((r, false));
            }
        }
        self.flags = 0;
        self.option_alone = false;
        v
    }
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

/// The Shift key of `NSEventModifierFlagShift`.
pub const SHIFT: u64 = 1 << 17;
/// The Control key of `NSEventModifierFlagControl`.
pub const CONTROL: u64 = 1 << 18;
/// The Option key of `NSEventModifierFlagOption`. The mouse rules use it too.
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
    use super::{
        COMMAND, CONTROL, Held, OPTION, SHIFT, Scroll, buttons, is_modifier, k, key_rune,
        rune_utf8, screen_point,
    };

    #[test]
    fn wayland_comes_before_x11_and_h9_gui_chooses() {
        use super::{LinuxBackEnd, linux_back_end};
        let env = |vars: &'static [(&'static str, &'static str)]| {
            move |v: &str| {
                vars.iter()
                    .find(|(k, _)| *k == v)
                    .map(|(_, x)| (*x).to_string())
            }
        };
        let both = env(&[("WAYLAND_DISPLAY", "wayland-1"), ("DISPLAY", ":0")]);
        assert_eq!(linux_back_end(&both), Some(LinuxBackEnd::Wayland));
        let x = env(&[
            ("WAYLAND_DISPLAY", "wayland-1"),
            ("DISPLAY", ":0"),
            ("H9_GUI", "x11"),
        ]);
        assert_eq!(linux_back_end(&x), Some(LinuxBackEnd::X11));
        let x = env(&[("WAYLAND_DISPLAY", ""), ("DISPLAY", ":0")]);
        assert_eq!(linux_back_end(&x), Some(LinuxBackEnd::X11));
        let none = env(&[("DISPLAY", ":0"), ("H9_GUI", "wayland")]);
        assert_eq!(linux_back_end(&none), None);
        assert_eq!(linux_back_end(&env(&[])), None);
    }

    #[test]
    fn a_key_up_is_for_the_rune_of_the_key_down() {
        let mut h = Held::default();
        let a = u32::from(b'A');
        assert_eq!(h.modifiers(SHIFT), [(k::SHIFT, true)]);
        assert_eq!(h.press(4, a), [(a, true)]);
        assert_eq!(h.modifiers(0), [(k::SHIFT, false)]);
        assert_eq!(h.release(4), [(a, false)]);
        assert_eq!(h.release(4), []);
    }

    #[test]
    fn a_repeat_of_the_host_is_one_more_key_down() {
        let mut h = Held::default();
        assert_eq!(h.press(7, k::UP), [(k::UP, true)]);
        assert_eq!(h.press(7, k::UP), [(k::UP, true)]);
        assert_eq!(h.press(8, 0), []);
        assert_eq!(h.release(8), []);
        assert_eq!(h.release(7), [(k::UP, false)]);
    }

    #[test]
    fn a_tap_of_option_is_kalt() {
        let mut h = Held::default();
        assert_eq!(h.modifiers(OPTION), []);
        assert_eq!(h.modifiers(0), [(k::ALT, true), (k::ALT, false)]);
        h.modifiers(OPTION);
        assert_eq!(h.press(1, 0xe9), [(0xe9, true)]);
        h.release(1);
        assert_eq!(h.modifiers(0), []);
        h.modifiers(OPTION);
        h.mouse_down();
        assert_eq!(h.modifiers(0), []);
        h.modifiers(OPTION);
        h.modifiers(OPTION | SHIFT);
        assert_eq!(h.modifiers(0), [(k::SHIFT, false)]);
    }

    #[test]
    fn the_keys_go_up_when_the_window_loses_the_keyboard() {
        let mut h = Held::default();
        let x = u32::from(b'x');
        assert_eq!(
            h.modifiers(CONTROL | COMMAND),
            [(k::CTL, true), (k::MOD4, true)]
        );
        h.press(2, x);
        assert_eq!(
            h.release_all(),
            [(x, false), (k::CTL, false), (k::MOD4, false)]
        );
        assert_eq!(h.release(2), []);
        assert_eq!(h.modifiers(0), []);
        assert!(is_modifier(k::ALTGR));
        assert!(!is_modifier(x));
    }

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
