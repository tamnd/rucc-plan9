//! The Wayland back end (plan 11.2). It speaks the Wayland protocol on the socket of the
//! compositor, so it needs no library, and the monitor starts on a host that has no Wayland.
//!
//! The window is an `xdg_toplevel` with the size of the Plan 9 screen. Its minimum and maximum
//! sizes are the size of the screen, so the compositor does not resize it. When the compositor
//! has `zxdg_decoration_manager_v1`, the window asks for the decorations of the server. The
//! screen goes to two `wl_shm` buffers of `XRGB8888`, which is the pixel of the screen, so no
//! pixel is converted. A frame that changed goes to a buffer that the compositor released, at
//! most 60 times a second.
//!
//! The key rules are the rules of the X11 back end. The compositor gives the keymap of the seat
//! as the text of an XKB keymap. The back end reads the keycodes and the symbols of the keys
//! from it, with the names of the keysyms of ASCII, Latin-1 and the keys that have no character.
//! Shift, Caps Lock, Control and the third level (Mod5, as for `AltGr`) come from the modifiers
//! event. Wayland has no auto-repeat in the server, so the back end repeats a key that is down
//! at the rate of `repeat_info`. Buttons 1, 2 and 3 are the buttons of `/dev/mouse`, and a click
//! of the wheel gives button 4 or 5 (8 or 16). The scroll of a touchpad gives a click for each
//! line, as on macOS.
//!
//! The clipboard does not go through this module: the snarf device of the monitor runs
//! `wl-paste` and `wl-copy`.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::collections::HashMap;

/// The keysym names of ASCII from 0x20, in order.
#[rustfmt::skip]
const ASCII: [&str; 95] = [
    "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand", "apostrophe",
    "parenleft", "parenright", "asterisk", "plus", "comma", "minus", "period", "slash", "0", "1",
    "2", "3", "4", "5", "6", "7", "8", "9", "colon", "semicolon", "less", "equal", "greater",
    "question", "at", "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O",
    "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z", "bracketleft", "backslash",
    "bracketright", "asciicircum", "underscore", "grave", "a", "b", "c", "d", "e", "f", "g", "h",
    "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
    "braceleft", "bar", "braceright", "asciitilde",
];

/// The keysym names of Latin-1 from 0xA0, in order.
#[rustfmt::skip]
const LATIN1: [&str; 96] = [
    "nobreakspace", "exclamdown", "cent", "sterling", "currency", "yen", "brokenbar", "section",
    "diaeresis", "copyright", "ordfeminine", "guillemotleft", "notsign", "hyphen", "registered",
    "macron", "degree", "plusminus", "twosuperior", "threesuperior", "acute", "mu", "paragraph",
    "periodcentered", "cedilla", "onesuperior", "masculine", "guillemotright", "onequarter",
    "onehalf", "threequarters", "questiondown", "Agrave", "Aacute", "Acircumflex", "Atilde",
    "Adiaeresis", "Aring", "AE", "Ccedilla", "Egrave", "Eacute", "Ecircumflex", "Ediaeresis",
    "Igrave", "Iacute", "Icircumflex", "Idiaeresis", "ETH", "Ntilde", "Ograve", "Oacute",
    "Ocircumflex", "Otilde", "Odiaeresis", "multiply", "Oslash", "Ugrave", "Uacute", "Ucircumflex",
    "Udiaeresis", "Yacute", "THORN", "ssharp", "agrave", "aacute", "acircumflex", "atilde",
    "adiaeresis", "aring", "ae", "ccedilla", "egrave", "eacute", "ecircumflex", "ediaeresis",
    "igrave", "iacute", "icircumflex", "idiaeresis", "eth", "ntilde", "ograve", "oacute",
    "ocircumflex", "otilde", "odiaeresis", "division", "oslash", "ugrave", "uacute", "ucircumflex",
    "udiaeresis", "yacute", "thorn", "ydiaeresis",
];

/// The other keysym names: the other names of some keys, and the keys that have no character.
const OTHER: &[(&str, u32)] = &[
    ("quoteright", 0x27),
    ("quoteleft", 0x60),
    ("guillemetleft", 0xab),
    ("guillemetright", 0xbb),
    ("ordmasculine", 0xba),
    ("Eth", 0xd0),
    ("Ooblique", 0xd8),
    ("Thorn", 0xde),
    ("ooblique", 0xf8),
    ("BackSpace", 0xff08),
    ("Tab", 0xff09),
    // Shift-Tab is a tab in Plan 9.
    ("ISO_Left_Tab", 0xff09),
    ("Return", 0xff0d),
    ("Escape", 0xff1b),
    ("Delete", 0xffff),
    ("Home", 0xff50),
    ("Left", 0xff51),
    ("Up", 0xff52),
    ("Right", 0xff53),
    ("Down", 0xff54),
    ("Prior", 0xff55),
    ("Page_Up", 0xff55),
    ("Next", 0xff56),
    ("Page_Down", 0xff56),
    ("End", 0xff57),
    ("Print", 0xff61),
    ("Insert", 0xff63),
    ("Mode_switch", 0xff7e),
    ("KP_Enter", 0xff8d),
    ("Shift_L", 0xffe1),
    ("Shift_R", 0xffe2),
    ("Control_L", 0xffe3),
    ("Control_R", 0xffe4),
    ("Alt_L", 0xffe9),
    ("Alt_R", 0xffea),
    ("Super_L", 0xffeb),
    ("Super_R", 0xffec),
    ("ISO_Level3_Shift", 0xfe03),
];

/// The keysym of the name `name` of an XKB keymap, or None for a name that the back end does not
/// know, such as `NoSymbol` and the dead keys.
#[must_use]
pub fn keysym(name: &str) -> Option<u32> {
    if let Some(i) = ASCII.iter().position(|n| *n == name) {
        return u32::try_from(i).ok().map(|i| 0x20 + i);
    }
    if let Some(i) = LATIN1.iter().position(|n| *n == name) {
        return u32::try_from(i).ok().map(|i| 0xa0 + i);
    }
    if let Some(&(_, v)) = OTHER.iter().find(|(n, _)| *n == name) {
        return Some(v);
    }
    if let Some(n) = name.strip_prefix("KP_")
        && let [d @ b'0'..=b'9'] = n.as_bytes()
    {
        return Some(0xffb0 + u32::from(d - b'0'));
    }
    if let Some(n) = name.strip_prefix('F')
        && let Ok(n @ 1..=12) = n.parse::<u32>()
    {
        return Some(0xffbe + n - 1);
    }
    // The keysym of a Unicode character is U and its code in hex, and Latin-1 is its own keysym.
    if let Some(h) = name.strip_prefix('U')
        && h.len() >= 4
        && let Ok(u) = u32::from_str_radix(h, 16)
    {
        return Some(if (0x20..0x7f).contains(&u) || (0xa0..=0xff).contains(&u) {
            u
        } else {
            0x0100_0000 + u
        });
    }
    if let Some(h) = name.strip_prefix("0x") {
        return u32::from_str_radix(h, 16).ok();
    }
    None
}

/// The tokens of the text of an XKB keymap: names, `<KEY>` names, strings, numbers and single
/// characters. The comments go.
fn tokens(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut v = Vec::new();
    let mut i = 0;
    let word =
        |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'+' || c == b'-' || c == b'.';
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'<' || c == b'"' {
            let end = if c == b'<' { b'>' } else { b'"' };
            i += 1;
            while i < b.len() && b[i] != end {
                i += 1;
            }
            i = (i + 1).min(b.len());
        } else if word(c) {
            while i < b.len() && word(b[i]) {
                i += 1;
            }
        } else {
            i += 1;
        }
        if let Some(t) = text.get(start..i) {
            v.push(t);
        }
    }
    v
}

/// The keysyms of the keys of an XKB keymap: for each keycode, the keysyms of each level of
/// each group.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Keymap {
    keys: HashMap<u32, Vec<Vec<u32>>>,
}

/// The index of the token after the token that closes the bracket at `at`, with the brackets
/// inside counted.
fn close(t: &[&str], at: usize) -> usize {
    let (open, shut) = match t.get(at) {
        Some(&"[") => ("[", "]"),
        Some(&"(") => ("(", ")"),
        _ => ("{", "}"),
    };
    let mut depth = 0;
    for (i, tok) in t.iter().enumerate().skip(at) {
        if *tok == open {
            depth += 1;
        } else if *tok == shut {
            depth -= 1;
            if depth == 0 {
                return i + 1;
            }
        }
    }
    t.len()
}

/// The keysyms of the list of names at `at`, which starts with `[`.
fn list(t: &[&str], at: usize) -> (Vec<u32>, usize) {
    let end = close(t, at);
    let syms = t[at + 1..end.saturating_sub(1)]
        .iter()
        .filter(|s| **s != ",")
        .map(|s| keysym(s).unwrap_or(0))
        .collect();
    (syms, end)
}

/// The groups of the symbols of the body of a `key` from `at` to `end`.
fn key_groups(t: &[&str], mut at: usize, end: usize) -> Vec<Vec<u32>> {
    let mut groups: Vec<Vec<u32>> = Vec::new();
    while at < end {
        match t[at] {
            "[" => {
                let (syms, next) = list(t, at);
                groups.push(syms);
                at = next;
            }
            name if name.starts_with(|c: char| c.is_ascii_alphabetic())
                && matches!(t.get(at + 1), Some(&"[" | &"=")) =>
            {
                at += 1;
                // The group of symbols[Group2] or symbols[2].
                let mut group = None;
                if t[at] == "[" {
                    let index = t.get(at + 1).copied().unwrap_or("");
                    group = index
                        .trim_start_matches("Group")
                        .parse::<usize>()
                        .ok()
                        .and_then(|g| g.checked_sub(1));
                    at = close(t, at);
                }
                if t.get(at) == Some(&"=") {
                    at += 1;
                }
                if name == "symbols" && t.get(at) == Some(&"[") {
                    let (syms, next) = list(t, at);
                    let g = group.unwrap_or(groups.len());
                    if groups.len() <= g {
                        groups.resize(g + 1, Vec::new());
                    }
                    groups[g] = syms;
                    at = next;
                } else if matches!(t.get(at), Some(&"[" | &"(")) {
                    at = close(t, at);
                } else {
                    while at < end && t[at] != "," {
                        at += if t[at] == "(" { close(t, at) - at } else { 1 };
                    }
                }
            }
            _ => at += 1,
        }
    }
    groups
}

impl Keymap {
    /// The keymap of the text of an XKB keymap of format `xkb_v1`. It reads the keycodes of
    /// `xkb_keycodes` (`<AE01> = 10;` and `alias <A> = <B>;`) and the keys of `xkb_symbols`
    /// (`key <AE01> { [ 1, exclam ] };`). The other parts of the keymap do not change a rune.
    #[must_use]
    pub fn parse(text: &str) -> Keymap {
        let t = tokens(text);
        let mut codes: HashMap<&str, u32> = HashMap::new();
        let mut aliases: HashMap<&str, &str> = HashMap::new();
        let mut keys = HashMap::new();
        let mut i = 0;
        while i < t.len() {
            let name = |j: usize| t.get(j).filter(|s| s.starts_with('<')).copied();
            if t[i] == "alias"
                && let (Some(a), Some(&"="), Some(b)) = (name(i + 1), t.get(i + 2), name(i + 3))
            {
                aliases.insert(a, b);
                i += 4;
            } else if let (Some(n), Some(&"="), Some(code)) = (name(i), t.get(i + 1), t.get(i + 2))
                && let Ok(code) = code.parse::<u32>()
            {
                codes.insert(n, code);
                i += 3;
            } else if t[i] == "key"
                && let (Some(n), Some(&"{")) = (name(i + 1), t.get(i + 2))
            {
                let end = close(&t, i + 2);
                let n = aliases.get(n).copied().unwrap_or(n);
                if let Some(&code) = codes.get(n) {
                    keys.insert(code, key_groups(&t, i + 3, end.saturating_sub(1)));
                }
                i = end;
            } else {
                i += 1;
            }
        }
        Keymap { keys }
    }

    /// The keysym of the XKB keycode `code` with the modifiers `mods` in the group `group`. With
    /// Shift, or with Caps Lock and a letter, it is the second level of the key. With Mod5, it is
    /// the third or the fourth level, when the key has them.
    #[must_use]
    pub fn keysym(&self, code: u32, mods: u32, group: u32) -> u32 {
        let Some(groups) = self.keys.get(&code) else {
            return 0;
        };
        let g = usize::try_from(group).unwrap_or(0);
        let Some(syms) = groups.get(g).or_else(|| groups.first()) else {
            return 0;
        };
        let level = |l: usize| syms.get(l).copied().unwrap_or(0);
        let base = if mods & MOD5 != 0 && level(2) != 0 {
            2
        } else {
            0
        };
        let (lower, upper) = (level(base), level(base + 1));
        let letter = (0x61..=0x7a).contains(&lower);
        let shifted = mods & SHIFT != 0 || (mods & LOCK != 0 && letter);
        if shifted {
            if upper != 0 {
                return upper;
            }
            if letter {
                return lower - 0x20;
            }
        }
        lower
    }
}

/// The real modifiers of XKB, as the modifiers event gives them. Shift, Lock and Control have
/// the bits of the state of X11, so `keysym_rune` takes the low bits as they are.
const SHIFT: u32 = 1;
const LOCK: u32 = 2;
const MOD5: u32 = 0x80;

/// The button bit of `/dev/mouse` for the Linux button code of a `wl_pointer` event: the left
/// (0x110), right (0x111) and middle (0x112) buttons are 1, 4 and 2.
#[must_use]
pub fn button_bit(code: u32) -> u8 {
    match code {
        0x110 => 1,
        0x111 => 4,
        0x112 => 2,
        _ => 0,
    }
}

/// A request to the compositor: the arguments, then the header when it is done.
#[derive(Debug, Default)]
pub(crate) struct Args(Vec<u8>);

impl Args {
    pub(crate) fn u(mut self, v: u32) -> Args {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    pub(crate) fn i(self, v: i32) -> Args {
        self.u(v.cast_unsigned())
    }

    /// A string: its length with the NUL, the bytes, the NUL, and the padding to 4 bytes.
    pub(crate) fn s(mut self, v: &str) -> Args {
        let n = u32::try_from(v.len() + 1).unwrap_or(u32::MAX);
        self.0.extend_from_slice(&n.to_le_bytes());
        self.0.extend_from_slice(v.as_bytes());
        self.0.push(0);
        while !self.0.len().is_multiple_of(4) {
            self.0.push(0);
        }
        self
    }

    /// The message of the request `op` on the object `id`.
    pub(crate) fn on(self, id: u32, op: u16) -> Vec<u8> {
        let size = u32::try_from(8 + self.0.len()).unwrap_or(0);
        let mut m = Vec::with_capacity(8 + self.0.len());
        m.extend_from_slice(&id.to_le_bytes());
        m.extend_from_slice(&(size << 16 | u32::from(op)).to_le_bytes());
        m.extend_from_slice(&self.0);
        m
    }
}

/// An event from the compositor: the object, the opcode and the arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    /// The object.
    pub id: u32,
    /// The opcode.
    pub op: u16,
    /// The bytes of the arguments.
    pub args: Vec<u8>,
}

/// A reader of the arguments of an event. A read past the end gives 0 or an empty value.
pub(crate) struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    pub(crate) fn u(&mut self) -> u32 {
        let v = self
            .b
            .get(self.at..self.at + 4)
            .map_or(0, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]));
        self.at += 4;
        v
    }

    fn i(&mut self) -> i32 {
        self.u().cast_signed()
    }

    /// A fixed point number of 24.8 bits.
    fn fixed(&mut self) -> f64 {
        f64::from(self.i()) / 256.0
    }

    /// The bytes of a string or an array, with the padding skipped.
    fn bytes(&mut self) -> &[u8] {
        let n = usize::try_from(self.u()).unwrap_or(0);
        let start = self.at.min(self.b.len());
        let end = (self.at + n).min(self.b.len());
        self.at += n.div_ceil(4) * 4;
        &self.b[start..end]
    }

    pub(crate) fn s(&mut self) -> String {
        let b = self.bytes();
        let b = b.strip_suffix(&[0]).unwrap_or(b);
        String::from_utf8_lossy(b).into_owned()
    }
}

impl Event {
    pub(crate) fn reader(&self) -> Reader<'_> {
        Reader {
            b: &self.args,
            at: 0,
        }
    }
}

/// Take the whole events from the front of `buf`.
pub(crate) fn split(buf: &mut Vec<u8>) -> Vec<Event> {
    let mut events = Vec::new();
    let mut at = 0;
    while let Some(h) = buf.get(at..at + 8) {
        let id = u32::from_le_bytes([h[0], h[1], h[2], h[3]]);
        let word = u32::from_le_bytes([h[4], h[5], h[6], h[7]]);
        let size = usize::try_from(word >> 16).unwrap_or(0).max(8);
        let Some(args) = buf.get(at + 8..at + size) else {
            break;
        };
        events.push(Event {
            id,
            op: u16::try_from(word & 0xffff).unwrap_or(0),
            args: args.to_vec(),
        });
        at += size;
    }
    buf.drain(..at);
    events
}

/// The path of the socket of the compositor: `WAYLAND_DISPLAY`, or `wayland-0`, in
/// `XDG_RUNTIME_DIR`. A `WAYLAND_DISPLAY` that is a full path is the path.
#[must_use]
pub fn socket_path(display: Option<&str>, runtime: Option<&str>) -> Option<std::path::PathBuf> {
    let d = display.filter(|d| !d.is_empty()).unwrap_or("wayland-0");
    if d.starts_with('/') {
        return Some(d.into());
    }
    runtime
        .filter(|r| !r.is_empty())
        .map(|r| std::path::Path::new(r).join(d))
}

#[cfg(target_os = "linux")]
pub use conn::run;
#[cfg(target_os = "linux")]
pub(crate) use conn::send_fd;

#[cfg(target_os = "linux")]
mod conn {
    use super::{Args, Event, Keymap, button_bit, split};
    use crate::x11::keysym_rune;
    use crate::{Held, Host, Scroll, is_modifier};
    use std::collections::VecDeque;
    use std::io::Write;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
    use std::os::unix::fs::FileExt;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
    use std::time::{Duration, Instant};

    /// The time between two frames.
    const FRAME: Duration = Duration::from_millis(16);

    /// The largest keymap that the back end reads.
    const KEYMAP_MAX: usize = 1 << 20;

    /// What the thread that reads the socket gives.
    enum Item {
        Event(Event),
        /// A file descriptor. It comes before the event that has it.
        Fd(OwnedFd),
        /// The connection ended.
        Gone,
    }

    /// Read from the socket into `buf`, and put the file descriptors that come with the bytes
    /// in `fds`. Give the count of the bytes, or 0 at the end.
    fn recv(fd: RawFd, buf: &mut [u8], fds: &mut Vec<OwnedFd>) -> std::io::Result<usize> {
        let mut iov = libc::iovec {
            iov_base: buf.as_mut_ptr().cast(),
            iov_len: buf.len(),
        };
        // Space for 28 descriptors, the most that libwayland sends in one message.
        let mut control = [0u64; 16];
        // SAFETY: msghdr is a plain C struct, and all zeros is a valid value.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        msg.msg_controllen = std::mem::size_of_val(&control);
        // SAFETY: msg points at the buffer and at the control space, which live until the end
        // of the call.
        let n = unsafe { libc::recvmsg(fd, &raw mut msg, libc::MSG_CMSG_CLOEXEC) };
        if n < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the kernel filled the control space, and CMSG_FIRSTHDR and CMSG_NXTHDR stay
        // inside msg_controllen.
        let mut c = unsafe { libc::CMSG_FIRSTHDR(&raw const msg) };
        while !c.is_null() {
            // SAFETY: c points at a control message inside the control space.
            let h = unsafe { &*c };
            if h.cmsg_level == libc::SOL_SOCKET && h.cmsg_type == libc::SCM_RIGHTS {
                // SAFETY: CMSG_LEN is arithmetic on a length.
                let head = unsafe { libc::CMSG_LEN(0) } as usize;
                let count = (h.cmsg_len as usize).saturating_sub(head) / size_of::<RawFd>();
                // SAFETY: CMSG_DATA points at the descriptors of the message.
                let data = unsafe { libc::CMSG_DATA(c) }.cast::<RawFd>();
                for k in 0..count {
                    // SAFETY: the message has count descriptors, which the kernel gave to this
                    // process, so each is owned here.
                    fds.push(unsafe { OwnedFd::from_raw_fd(data.add(k).read_unaligned()) });
                }
            }
            // SAFETY: c is a control message of msg.
            c = unsafe { libc::CMSG_NXTHDR(&raw const msg, c) };
        }
        Ok(usize::try_from(n).unwrap_or(0))
    }

    /// Send `data` with the file descriptor `fd`.
    pub(crate) fn send_fd(sock: &UnixStream, data: &[u8], fd: RawFd) -> Result<(), String> {
        let mut iov = libc::iovec {
            iov_base: data.as_ptr().cast_mut().cast(),
            iov_len: data.len(),
        };
        let mut control = [0u64; 4];
        // SAFETY: msghdr is a plain C struct, and all zeros is a valid value.
        let mut msg: libc::msghdr = unsafe { std::mem::zeroed() };
        msg.msg_iov = &raw mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = control.as_mut_ptr().cast();
        // SAFETY: CMSG_SPACE is arithmetic on a length.
        msg.msg_controllen = unsafe { libc::CMSG_SPACE(size_of::<RawFd>() as u32) } as usize;
        // SAFETY: the control space has room for one message with one descriptor.
        unsafe {
            let c = libc::CMSG_FIRSTHDR(&raw const msg);
            (*c).cmsg_level = libc::SOL_SOCKET;
            (*c).cmsg_type = libc::SCM_RIGHTS;
            (*c).cmsg_len = libc::CMSG_LEN(size_of::<RawFd>() as u32) as usize;
            libc::CMSG_DATA(c).cast::<RawFd>().write_unaligned(fd);
        }
        // SAFETY: msg points at data and at the control space, which live until the end of the
        // call.
        let n = unsafe { libc::sendmsg(sock.as_raw_fd(), &raw const msg, libc::MSG_NOSIGNAL) };
        if usize::try_from(n).ok() == Some(data.len()) {
            Ok(())
        } else {
            Err(format!(
                "cannot send to the compositor: {}",
                std::io::Error::last_os_error()
            ))
        }
    }

    /// Shared memory for the buffers of the screen.
    struct Shm {
        fd: OwnedFd,
        ptr: *mut u8,
        len: usize,
    }

    impl Shm {
        fn new(len: usize) -> Result<Shm, String> {
            let err = |what: &str| format!("{what}: {}", std::io::Error::last_os_error());
            // SAFETY: the name is a C string, and the call makes a new descriptor.
            let raw = unsafe { libc::memfd_create(c"9h-screen".as_ptr(), libc::MFD_CLOEXEC) };
            if raw < 0 {
                return Err(err("memfd_create"));
            }
            // SAFETY: memfd_create gave a new descriptor that nothing else owns.
            let fd = unsafe { OwnedFd::from_raw_fd(raw) };
            let size = libc::off_t::try_from(len).map_err(|_| "the screen is too large")?;
            // SAFETY: fd is an open memfd.
            if unsafe { libc::ftruncate(fd.as_raw_fd(), size) } < 0 {
                return Err(err("ftruncate"));
            }
            // SAFETY: a new shared mapping of len bytes of fd, which has len bytes.
            let ptr = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    len,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    fd.as_raw_fd(),
                    0,
                )
            };
            if ptr == libc::MAP_FAILED {
                return Err(err("mmap"));
            }
            Ok(Shm {
                fd,
                ptr: ptr.cast(),
                len,
            })
        }

        /// Copy the pixels to the byte `at` of the memory.
        fn put(&mut self, at: usize, pixels: &[u32]) {
            let n = std::mem::size_of_val(pixels).min(self.len.saturating_sub(at));
            // SAFETY: the copy stays inside the mapping, and the pixels do not overlap it.
            unsafe {
                std::ptr::copy_nonoverlapping(pixels.as_ptr().cast::<u8>(), self.ptr.add(at), n);
            }
        }
    }

    impl Drop for Shm {
        fn drop(&mut self) {
            // SAFETY: ptr and len are the mapping of new.
            unsafe {
                libc::munmap(self.ptr.cast(), self.len);
            }
        }
    }

    /// The two buffers of the screen, in one pool.
    struct Buffers {
        shm: Shm,
        pool: u32,
        ids: [u32; 2],
        busy: [bool; 2],
        w: u32,
        h: u32,
    }

    /// The globals of the compositor that the window uses: the name and the version.
    #[derive(Default)]
    struct Globals {
        compositor: Option<(u32, u32)>,
        shm: Option<(u32, u32)>,
        wm_base: Option<(u32, u32)>,
        seat: Option<(u32, u32)>,
        decoration: Option<(u32, u32)>,
    }

    /// A key that repeats: the keycode, its rune, and the time of the next repeat.
    struct Repeat {
        code: u32,
        rune: u32,
        next: Instant,
    }

    /// The connection and the state of the window.
    struct Client<'h> {
        sock: UnixStream,
        rx: Receiver<Item>,
        fds: VecDeque<OwnedFd>,
        next_id: u32,
        host: &'h dyn Host,
        registry: u32,
        globals: Globals,
        compositor: u32,
        shm: u32,
        wm_base: u32,
        seat: u32,
        seat_version: u32,
        surface: u32,
        xdg_surface: u32,
        toplevel: u32,
        pointer: u32,
        keyboard: u32,
        configured: bool,
        buffers: Option<Buffers>,
        keymap: Keymap,
        mods: u32,
        group: u32,
        held: Held,
        rate: u32,
        delay: u32,
        repeat: Option<Repeat>,
        buttons: u8,
        x: f64,
        y: f64,
        w: u32,
        h: u32,
        scroll: Scroll,
        discrete: i32,
        value: f64,
        error: Option<String>,
    }

    impl Client<'_> {
        fn id(&mut self) -> u32 {
            let id = self.next_id;
            self.next_id += 1;
            id
        }

        fn send(&self, m: &[u8]) -> Result<(), String> {
            (&self.sock)
                .write_all(m)
                .map_err(|e| format!("cannot send to the compositor: {e}"))
        }

        /// Bind the global `g` with at most the version `max`, and give the new object and the
        /// version.
        fn bind(&mut self, g: (u32, u32), iface: &str, max: u32) -> Result<(u32, u32), String> {
            let id = self.id();
            let v = g.1.min(max);
            self.send(
                &Args::default()
                    .u(g.0)
                    .s(iface)
                    .u(v)
                    .u(id)
                    .on(self.registry, 0),
            )?;
            Ok((id, v))
        }

        /// Wait until the compositor did each request before this one, and handle the events
        /// that come before it.
        fn roundtrip(&mut self) -> Result<(), String> {
            let cb = self.id();
            self.send(&Args::default().u(cb).on(1, 0))?;
            loop {
                match self.rx.recv() {
                    Ok(Item::Event(e)) if e.id == cb => return Ok(()),
                    Ok(item) => self.item(item)?,
                    Err(_) => return Err("the compositor closed the connection".into()),
                }
            }
        }

        /// Handle an item of the thread that reads the socket.
        fn item(&mut self, item: Item) -> Result<(), String> {
            match item {
                Item::Fd(fd) => self.fds.push_back(fd),
                Item::Event(e) => self.event(&e)?,
                Item::Gone => return Err("the compositor closed the connection".into()),
            }
            match self.error.take() {
                Some(e) => Err(e),
                None => Ok(()),
            }
        }

        fn give(&self, events: Vec<crate::KeyEvent>) {
            for (r, down) in events {
                self.host.key(r, down);
            }
        }

        /// The point of the screen of the point of the surface.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)] // Clamped first.
        fn point(&self) -> (u16, u16) {
            let c = |v: f64, n: u32| v.clamp(0.0, f64::from(n.saturating_sub(1))) as u16;
            (c(self.x, self.w), c(self.y, self.h))
        }

        /// Give the clicks of the wheel.
        fn wheel(&self, clicks: &[u8]) {
            let (x, y) = self.point();
            for &bit in clicks {
                self.host.mouse(x, y, self.buttons | bit);
                self.host.mouse(x, y, self.buttons);
            }
        }

        /// Give the scroll of the last frame of the pointer.
        fn axis_frame(&mut self) {
            let clicks: Vec<u8> = if self.discrete != 0 {
                let bit = if self.discrete < 0 { 8 } else { 16 };
                vec![bit; usize::try_from(self.discrete.unsigned_abs()).unwrap_or(0)]
            } else if self.value != 0.0 {
                // The scroll of Wayland is down for a positive value, and Scroll is up for one.
                self.scroll.add(-self.value, true)
            } else {
                vec![]
            };
            self.discrete = 0;
            self.value = 0.0;
            self.wheel(&clicks);
        }

        #[allow(clippy::too_many_lines)] // One arm for each event of the window.
        fn event(&mut self, e: &Event) -> Result<(), String> {
            let mut r = e.reader();
            match (e.id, e.op) {
                (1, 0) => {
                    let (object, code) = (r.u(), r.u());
                    self.error = Some(format!(
                        "the compositor gave error {code} on object {object}: {}",
                        r.s()
                    ));
                }
                (id, 0) if id == self.registry => {
                    let (name, iface, version) = (r.u(), r.s(), r.u());
                    let g = Some((name, version));
                    match iface.as_str() {
                        "wl_compositor" => self.globals.compositor = g,
                        "wl_shm" => self.globals.shm = g,
                        "xdg_wm_base" => self.globals.wm_base = g,
                        "wl_seat" if self.globals.seat.is_none() => self.globals.seat = g,
                        "zxdg_decoration_manager_v1" => self.globals.decoration = g,
                        _ => {}
                    }
                }
                (id, 0) if id == self.wm_base => {
                    let serial = r.u();
                    self.send(&Args::default().u(serial).on(self.wm_base, 3))?;
                }
                (id, 0) if id == self.xdg_surface => {
                    let serial = r.u();
                    self.send(&Args::default().u(serial).on(self.xdg_surface, 4))?;
                    if !self.configured {
                        self.configured = true;
                        // The first frame comes again.
                        if let Some(b) = &mut self.buffers {
                            b.busy = [false; 2];
                        }
                    }
                }
                (id, 1) if id == self.toplevel => self.host.quit(),
                (id, 0) if id == self.seat => {
                    let caps = r.u();
                    if caps & 1 != 0 && self.pointer == 0 {
                        self.pointer = self.id();
                        self.send(&Args::default().u(self.pointer).on(self.seat, 0))?;
                    }
                    if caps & 2 != 0 && self.keyboard == 0 {
                        self.keyboard = self.id();
                        self.send(&Args::default().u(self.keyboard).on(self.seat, 1))?;
                    }
                }
                (id, op) if id == self.pointer && id != 0 => match op {
                    0 => {
                        let _serial = r.u();
                        let _surface = r.u();
                        (self.x, self.y) = (r.fixed(), r.fixed());
                        let (x, y) = self.point();
                        self.host.mouse(x, y, self.buttons);
                    }
                    2 => {
                        let _time = r.u();
                        (self.x, self.y) = (r.fixed(), r.fixed());
                        let (x, y) = self.point();
                        self.host.mouse(x, y, self.buttons);
                    }
                    3 => {
                        let (_serial, _time, code, state) = (r.u(), r.u(), r.u(), r.u());
                        let bit = button_bit(code);
                        if state == 1 {
                            self.buttons |= bit;
                        } else {
                            self.buttons &= !bit;
                        }
                        let (x, y) = self.point();
                        self.host.mouse(x, y, self.buttons);
                    }
                    4 => {
                        let (_time, axis, value) = (r.u(), r.u(), r.fixed());
                        if axis == 0 {
                            self.value += value;
                            // A seat before version 5 has no frame event, and each event is a
                            // click.
                            if self.seat_version < 5 {
                                self.discrete = if value < 0.0 { -1 } else { 1 };
                                self.axis_frame();
                            }
                        }
                    }
                    5 => self.axis_frame(),
                    8 => {
                        let (axis, n) = (r.u(), r.i());
                        if axis == 0 {
                            self.discrete += n;
                        }
                    }
                    _ => {}
                },
                (id, op) if id == self.keyboard && id != 0 => {
                    match op {
                        0 => {
                            let (format, size) = (r.u(), r.u());
                            let fd = self.fds.pop_front();
                            if let Some(fd) = fd
                                && format == 1
                            {
                                let n = usize::try_from(size).unwrap_or(0).min(KEYMAP_MAX);
                                let mut text = vec![0u8; n];
                                let file = std::fs::File::from(fd);
                                if file.read_exact_at(&mut text, 0).is_ok() {
                                    let text = String::from_utf8_lossy(&text);
                                    self.keymap = Keymap::parse(text.trim_end_matches('\0'));
                                }
                            }
                        }
                        2 => {
                            self.repeat = None;
                            let up = self.held.release_all();
                            self.give(up);
                        }
                        3 => {
                            let (_serial, _time, key, state) = (r.u(), r.u(), r.u(), r.u());
                            let code = key + 8;
                            if state == 1 {
                                let sym = self.keymap.keysym(code, self.mods, self.group);
                                let mods = u16::try_from(self.mods & 0xff).unwrap_or(0);
                                let rune = keysym_rune(sym, mods).unwrap_or(0);
                                let down = self.held.press(code, rune);
                                self.give(down);
                                self.repeat = (rune != 0 && !is_modifier(rune) && self.rate > 0)
                                    .then(|| Repeat {
                                        code,
                                        rune,
                                        next: Instant::now()
                                            + Duration::from_millis(u64::from(self.delay)),
                                    });
                            } else {
                                if self.repeat.as_ref().is_some_and(|k| k.code == code) {
                                    self.repeat = None;
                                }
                                let up = self.held.release(code);
                                self.give(up);
                            }
                        }
                        4 => {
                            let (_serial, depressed, latched, locked, group) =
                                (r.u(), r.u(), r.u(), r.u(), r.u());
                            self.mods = depressed | latched | locked;
                            self.group = group;
                        }
                        5 => {
                            let (rate, delay) = (r.i(), r.i());
                            self.rate = u32::try_from(rate).unwrap_or(0);
                            self.delay = u32::try_from(delay).unwrap_or(0);
                        }
                        _ => {}
                    }
                }
                (id, 0) => {
                    if let Some(b) = &mut self.buffers
                        && let Some(i) = b.ids.iter().position(|x| *x == id)
                    {
                        b.busy[i] = false;
                    }
                }
                _ => {}
            }
            Ok(())
        }

        /// Make the buffers for a screen of `w` by `h` pixels.
        fn buffers(&mut self, w: u32, h: u32) -> Result<(), String> {
            if let Some(old) = self.buffers.take() {
                for id in old.ids {
                    self.send(&Args::default().on(id, 0))?;
                }
                self.send(&Args::default().on(old.pool, 1))?;
            }
            let stride = w * 4;
            let frame = usize::try_from(u64::from(stride) * u64::from(h))
                .map_err(|_| "the screen is too large")?;
            let shm = Shm::new(frame * 2)?;
            let pool = self.id();
            let size = i32::try_from(frame * 2).map_err(|_| "the screen is too large")?;
            send_fd(
                &self.sock,
                &Args::default().u(pool).i(size).on(self.shm, 0),
                shm.fd.as_raw_fd(),
            )?;
            let mut ids = [0; 2];
            for (n, id) in ids.iter_mut().enumerate() {
                *id = self.id();
                let at = i32::try_from(frame * n).map_err(|_| "the screen is too large")?;
                let dim = |v: u32| i32::try_from(v).unwrap_or(0);
                // The format 1 is XRGB8888.
                let m = Args::default()
                    .u(*id)
                    .i(at)
                    .i(dim(w))
                    .i(dim(h))
                    .i(dim(stride))
                    .u(1)
                    .on(pool, 0);
                self.send(&m)?;
            }
            // The size of the window is the size of the screen.
            let size = Args::default().i(i32::try_from(w).unwrap_or(0));
            let size = size.i(i32::try_from(h).unwrap_or(0));
            self.send(&Args(size.0.clone()).on(self.toplevel, 7))?;
            self.send(&size.on(self.toplevel, 8))?;
            self.buffers = Some(Buffers {
                shm,
                pool,
                ids,
                busy: [false; 2],
                w,
                h,
            });
            (self.w, self.h) = (w, h);
            Ok(())
        }

        /// Show the pixels of a screen of `w` by `h` pixels. Give false when no buffer is free,
        /// and then the frame must come again.
        fn show(&mut self, w: u32, h: u32, pixels: &[u32]) -> Result<bool, String> {
            if self.buffers.as_ref().is_none_or(|b| (b.w, b.h) != (w, h)) {
                self.buffers(w, h)?;
            }
            if !self.configured {
                return Ok(false);
            }
            let Some(b) = &mut self.buffers else {
                return Ok(false);
            };
            let Some(i) = b.busy.iter().position(|busy| !busy) else {
                return Ok(false);
            };
            let frame = usize::try_from(u64::from(w) * u64::from(h) * 4).unwrap_or(0);
            b.shm.put(frame * i, pixels);
            b.busy[i] = true;
            let id = b.ids[i];
            let (wi, hi) = (i32::try_from(w).unwrap_or(0), i32::try_from(h).unwrap_or(0));
            self.send(&Args::default().u(id).i(0).i(0).on(self.surface, 1))?;
            self.send(&Args::default().i(0).i(0).i(wi).i(hi).on(self.surface, 2))?;
            self.send(&Args::default().on(self.surface, 6))?;
            Ok(true)
        }
    }

    /// Show a window of `width` by `height` pixels with the title `title` on the compositor of
    /// `WAYLAND_DISPLAY`, and run the event loop until the session ends.
    #[allow(clippy::too_many_lines)] // The setup of the window, then the loop.
    pub fn run(title: &str, width: u32, height: u32, host: &dyn Host) -> Result<(), String> {
        let env = |v: &str| std::env::var(v).ok();
        let path = super::socket_path(
            env("WAYLAND_DISPLAY").as_deref(),
            env("XDG_RUNTIME_DIR").as_deref(),
        )
        .ok_or("XDG_RUNTIME_DIR is not set")?;
        let sock = UnixStream::connect(&path)
            .map_err(|e| format!("cannot connect to {}: {e}", path.display()))?;
        let reader = sock.try_clone().map_err(|e| e.to_string())?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("9h-wayland".into())
            .spawn(move || {
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 4096];
                loop {
                    let mut fds = Vec::new();
                    match recv(reader.as_raw_fd(), &mut chunk, &mut fds) {
                        Ok(0) => break,
                        Ok(n) => {
                            for fd in fds {
                                let _ = tx.send(Item::Fd(fd));
                            }
                            buf.extend_from_slice(&chunk[..n]);
                            for e in split(&mut buf) {
                                if tx.send(Item::Event(e)).is_err() {
                                    return;
                                }
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => break,
                    }
                }
                let _ = tx.send(Item::Gone);
            })
            .map_err(|e| e.to_string())?;
        let mut c = Client {
            sock,
            rx,
            fds: VecDeque::new(),
            next_id: 2,
            host,
            registry: 0,
            globals: Globals::default(),
            compositor: 0,
            shm: 0,
            wm_base: 0,
            seat: 0,
            seat_version: 0,
            surface: 0,
            xdg_surface: 0,
            toplevel: 0,
            pointer: 0,
            keyboard: 0,
            configured: false,
            buffers: None,
            keymap: Keymap::default(),
            mods: 0,
            group: 0,
            held: Held::default(),
            rate: 25,
            delay: 600,
            repeat: None,
            buttons: 0,
            x: 0.0,
            y: 0.0,
            w: width,
            h: height,
            scroll: Scroll::default(),
            discrete: 0,
            value: 0.0,
            error: None,
        };
        c.registry = c.id();
        c.send(&Args::default().u(c.registry).on(1, 1))?;
        c.roundtrip()?;
        let need = |g: Option<(u32, u32)>, name: &str| {
            g.ok_or_else(|| format!("the compositor has no {name}"))
        };
        let g = need(c.globals.compositor, "wl_compositor")?;
        c.compositor = c.bind(g, "wl_compositor", 4)?.0;
        let g = need(c.globals.shm, "wl_shm")?;
        c.shm = c.bind(g, "wl_shm", 1)?.0;
        let g = need(c.globals.wm_base, "xdg_wm_base")?;
        c.wm_base = c.bind(g, "xdg_wm_base", 1)?.0;
        if let Some(g) = c.globals.seat {
            (c.seat, c.seat_version) = c.bind(g, "wl_seat", 7)?;
        }
        c.surface = c.id();
        c.send(&Args::default().u(c.surface).on(c.compositor, 0))?;
        c.xdg_surface = c.id();
        c.send(
            &Args::default()
                .u(c.xdg_surface)
                .u(c.surface)
                .on(c.wm_base, 2),
        )?;
        c.toplevel = c.id();
        c.send(&Args::default().u(c.toplevel).on(c.xdg_surface, 1))?;
        c.send(&Args::default().s(title).on(c.toplevel, 2))?;
        c.send(&Args::default().s("9h").on(c.toplevel, 3))?;
        if let Some(g) = c.globals.decoration {
            let (manager, _) = c.bind(g, "zxdg_decoration_manager_v1", 1)?;
            let decoration = c.id();
            c.send(&Args::default().u(decoration).u(c.toplevel).on(manager, 1))?;
            // The mode 2 is the decorations of the server.
            c.send(&Args::default().u(2).on(decoration, 1))?;
        }
        c.buffers(width, height)?;
        // The first commit has no buffer. The compositor answers it with a configure.
        c.send(&Args::default().on(c.surface, 6))?;
        c.roundtrip()?;

        let mut generation = 0u64;
        let mut pixels = Vec::new();
        let mut size: Option<(u32, u32)> = None;
        let mut due = false;
        let mut next = Instant::now();
        loop {
            if host.done() {
                return Ok(());
            }
            let now = Instant::now();
            if now >= next {
                if let Some(s) = host.frame(&mut generation, &mut pixels) {
                    size = Some(s);
                    due = true;
                }
                if due && let Some((w, h)) = size {
                    due = !c.show(w, h, &pixels)?;
                }
                next += FRAME;
                if next < now {
                    next = now + FRAME;
                }
            }
            if let Some(k) = &mut c.repeat
                && now >= k.next
            {
                k.next = now + Duration::from_millis(1000 / u64::from(c.rate.max(1)));
                let (code, rune) = (k.code, k.rune);
                let down = c.held.press(code, rune);
                c.give(down);
            }
            let mut wake = next;
            if let Some(k) = &c.repeat {
                wake = wake.min(k.next);
            }
            match c
                .rx
                .recv_timeout(wake.saturating_duration_since(Instant::now()))
            {
                Ok(item) => {
                    if let Err(e) = c.item(item) {
                        host.quit();
                        return Err(e);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    host.quit();
                    return Err("the compositor closed the connection".into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Args, Event, Keymap, keysym, socket_path, split};

    /// A part of the keymap that xkbcommon gives for the layout us, and a key of the layout de.
    const KEYMAP: &str = r#"xkb_keymap {
xkb_keycodes "evdev+aliases(qwerty)" {
	minimum = 8;
	maximum = 255;
	<ESC>                = 9;
	<AE01>               = 10;
	<AD01>               = 24;
	<AC01>               = 38;
	<LFSH>               = 50;
	<BKSL>               = 51;
	<AB10>               = 61;
	<RTRN>               = 36;
	<LCTL>               = 37;
	<RALT>               = 108;
	indicator 1 = "Caps Lock";
	alias <AC12>         = <BKSL>;
};
xkb_types "complete" {
	virtual_modifiers NumLock,Alt,LevelThree;
	type "ALPHABETIC" {
		modifiers= Shift+Lock;
		map[Shift]= Level2;
		level_name[Level1]= "Base";
	};
};
xkb_compat "complete" {
	interpret Shift_L+AnyOf(all) {
		action= SetMods(modifiers=Shift,clearLocks);
	};
};
xkb_symbols "pc+us+inet(evdev)" {
	name[Group1]="English (US)";
	key <ESC>                {	[          Escape ] };
	key <AE01>               {	[               1,          exclam ] };
	key <AD01>               {	[               q,               Q,              at ] };
	key <AC01>               {
		type= "ALPHABETIC",
		symbols[Group1]= [               a,               A ]
	};
	key <LFSH>               {	[         Shift_L ] };
	key <AC12>               {	[       backslash,             bar ] };
	key <AB10>               {	[           slash,        question ], [ minus, underscore ] };
	key <RTRN>               {	[          Return ] };
	key <LCTL>               {	[       Control_L ] };
	key <RALT>               {
		type= "TWO_LEVEL",
		symbols[Group1]= [ ISO_Level3_Shift ],
		actions[Group1]= [ SetMods(modifiers=LevelThree,clearLocks) ]
	};
	modifier_map Shift { <LFSH> };
};
};
"#;

    #[test]
    fn the_names_of_the_keysyms() {
        assert_eq!(keysym("a"), Some(0x61));
        assert_eq!(keysym("exclam"), Some(0x21));
        assert_eq!(keysym("asciitilde"), Some(0x7e));
        assert_eq!(keysym("eacute"), Some(0xe9));
        assert_eq!(keysym("ydiaeresis"), Some(0xff));
        assert_eq!(keysym("Return"), Some(0xff0d));
        assert_eq!(keysym("F12"), Some(0xffc9));
        assert_eq!(keysym("KP_7"), Some(0xffb7));
        assert_eq!(keysym("U20AC"), Some(0x0100_20ac));
        assert_eq!(keysym("U00E9"), Some(0xe9));
        assert_eq!(keysym("0x1000041"), Some(0x0100_0041));
        assert_eq!(keysym("NoSymbol"), None);
        assert_eq!(keysym("dead_acute"), None);
        assert_eq!(keysym("F13"), None);
    }

    #[test]
    fn the_keymap_gives_the_keysym_of_each_level() {
        let k = Keymap::parse(KEYMAP);
        assert_eq!(k.keysym(9, 0, 0), 0xff1b);
        assert_eq!(k.keysym(10, 0, 0), u32::from('1'));
        assert_eq!(k.keysym(10, 1, 0), u32::from('!'));
        assert_eq!(k.keysym(38, 0, 0), u32::from('a'));
        assert_eq!(k.keysym(38, 1, 0), u32::from('A'));
        // Caps Lock gives the capital of a letter, and not the second level of other keys.
        assert_eq!(k.keysym(38, 2, 0), u32::from('A'));
        assert_eq!(k.keysym(10, 2, 0), u32::from('1'));
        // The third level with Mod5.
        assert_eq!(k.keysym(24, 0x80, 0), u32::from('@'));
        assert_eq!(k.keysym(24, 1, 0), u32::from('Q'));
        // An alias, and a key of two groups.
        assert_eq!(k.keysym(51, 1, 0), u32::from('|'));
        assert_eq!(k.keysym(61, 1, 0), u32::from('?'));
        assert_eq!(k.keysym(61, 0, 1), u32::from('-'));
        assert_eq!(k.keysym(61, 0, 5), u32::from('/'));
        // The actions of a key are not symbols.
        assert_eq!(k.keysym(108, 0, 0), 0xfe03);
        assert_eq!(k.keysym(50, 0, 0), 0xffe1);
        assert_eq!(k.keysym(37, 0, 0), 0xffe3);
        assert_eq!(k.keysym(200, 0, 0), 0);
    }

    #[test]
    fn a_request_has_its_size_and_a_padded_string() {
        let m = Args::default().u(7).s("wl_shm").u(1).on(2, 0);
        assert_eq!(m.len(), 8 + 4 + 4 + 8 + 4);
        assert_eq!(&m[..4], &2u32.to_le_bytes());
        assert_eq!(&m[4..8], &(28u32 << 16).to_le_bytes());
        assert_eq!(&m[12..16], &7u32.to_le_bytes());
        assert_eq!(&m[16..24], b"wl_shm\0\0");
        let m = Args::default().i(-1).on(5, 3);
        assert_eq!(&m[4..8], &((12u32 << 16) | 3).to_le_bytes());
        assert_eq!(&m[8..], &[0xff; 4]);
    }

    #[test]
    fn the_events_are_split_and_a_part_stays() {
        let mut buf = Args::default().u(1).s("xdg_wm_base").u(5).on(2, 0);
        buf.extend(Args::default().u(9).on(3, 0));
        let whole = buf.len();
        buf.extend(&Args::default().u(1).on(4, 2)[..6]);
        let events = split(&mut buf);
        assert_eq!(buf.len(), 6);
        assert_eq!(events.len(), 2);
        assert_eq!(
            events[1],
            Event {
                id: 3,
                op: 0,
                args: 9u32.to_le_bytes().to_vec()
            }
        );
        let mut r = events[0].reader();
        assert_eq!((r.u(), r.s(), r.u()), (1, "xdg_wm_base".to_string(), 5));
        assert!(whole > 0);
    }

    #[test]
    fn the_socket_is_in_the_runtime_directory() {
        assert_eq!(
            socket_path(Some("wayland-1"), Some("/run/user/1000")),
            Some("/run/user/1000/wayland-1".into())
        );
        assert_eq!(
            socket_path(None, Some("/run/user/1000")),
            Some("/run/user/1000/wayland-0".into())
        );
        assert_eq!(socket_path(Some("/tmp/w"), None), Some("/tmp/w".into()));
        assert_eq!(socket_path(Some("wayland-1"), None), None);
    }

    /// With `H9_NEED_WAYLAND` set, the test needs a compositor on `WAYLAND_DISPLAY` that puts the
    /// window at 0,0 with no border, such as sway in CI, and `grim`. It shows an image in a
    /// window, and reads the pixels back with `grim`. With no `H9_NEED_WAYLAND` and no
    /// `WAYLAND_DISPLAY`, the test does nothing.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_window_shows_the_pixels_of_the_screen() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::time::{Duration, Instant};

        struct Fixed {
            pixels: Vec<u32>,
            w: u32,
            h: u32,
            stop: AtomicBool,
        }
        impl crate::Host for Fixed {
            fn frame(&self, generation: &mut u64, pixels: &mut Vec<u32>) -> Option<(u32, u32)> {
                if *generation == 1 {
                    return None;
                }
                *generation = 1;
                pixels.clone_from(&self.pixels);
                Some((self.w, self.h))
            }
            fn mouse(&self, _: u16, _: u16, _: u8) {}
            fn keys(&self, _: &[u8]) {}
            fn quit(&self) {
                self.stop.store(true, Ordering::SeqCst);
            }
            fn done(&self) -> bool {
                self.stop.load(Ordering::SeqCst)
            }
        }

        let need = std::env::var_os("H9_NEED_WAYLAND").is_some();
        if !need && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return;
        }
        let (w, h) = (300u32, 200u32);
        let host = Arc::new(Fixed {
            pixels: (0..w * h)
                .map(|i| i.wrapping_mul(2_654_435_761) & 0x00ff_ffff)
                .collect(),
            w,
            h,
            stop: AtomicBool::new(false),
        });
        let window = Arc::clone(&host);
        let t = std::thread::spawn(move || super::run("h9-gui test", w, h, &*window));
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let out = std::process::Command::new("grim")
                .args(["-g", &format!("0,0 {w}x{h}"), "-t", "ppm", "-"])
                .output()
                .expect("grim");
            let got = super::tests::ppm(&out.stdout);
            if got.as_deref() == Some(&host.pixels[..]) {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the window never showed the pixels: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        host.stop.store(true, Ordering::SeqCst);
        t.join().unwrap().unwrap();
    }

    /// The pixels of a binary PPM file, as 0x00RRGGBB.
    #[cfg(target_os = "linux")]
    fn ppm(b: &[u8]) -> Option<Vec<u32>> {
        // The header is P6, the width, the height and the largest value, then one space.
        let mut fields = 0;
        let mut at = 0;
        while fields < 4 {
            while b.get(at)?.is_ascii_whitespace() {
                at += 1;
            }
            while !b.get(at)?.is_ascii_whitespace() {
                at += 1;
            }
            fields += 1;
        }
        let data = b.get(at + 1..)?;
        Some(
            data.as_chunks::<3>()
                .0
                .iter()
                .map(|p| u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
                .collect(),
        )
    }
}
