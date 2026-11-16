//! The X11 back end (plan 11). It speaks the X11 protocol on the socket of the display, so it
//! needs no library, and the monitor starts on a host that has no X11.
//!
//! The window has the size of the Plan 9 screen, and the window manager is told not to resize
//! it. Each frame that changed goes to the window with `PutImage` in strips of rows that fit in
//! one request. The screen is `x8r8g8b8`, which is the pixel of a 24-bit `TrueColor` visual on a
//! little-endian host, so no pixel is converted.
//!
//! The key rules: the keysym of the key with Shift or Caps Lock gives a rune. Return is a new
//! line, `BackSpace` is `\b`, `Delete` is 0x7F, and the arrows and the function keys are the
//! runes of `keyboard.h`. With Control, a letter gives its control character. Shift, Control,
//! Alt, `AltGr` and Super give `Kshift`, `Kctl`, `Kalt`, `Kaltgr` and `Kmod4`. A key press is a
//! key down and a key release is a key up of the rune of the press. The auto-repeat of the
//! server gives a release and a press with the same time, which is one more key down. When the
//! window loses the focus, each key that is down goes up. Buttons 1, 2 and 3 are the buttons of
//! `/dev/mouse`, and the wheel gives buttons 4 and 5 (8 and 16).

use crate::{Held, Host, k};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

/// The time between two frames.
const FRAME: Duration = Duration::from_millis(16);

/// The request codes of the core protocol.
mod op {
    pub const CREATE_WINDOW: u8 = 1;
    pub const DESTROY_WINDOW: u8 = 4;
    pub const MAP_WINDOW: u8 = 8;
    pub const INTERN_ATOM: u8 = 16;
    pub const CHANGE_PROPERTY: u8 = 18;
    pub const CREATE_GC: u8 = 55;
    pub const PUT_IMAGE: u8 = 72;
    #[cfg(test)]
    pub const GET_IMAGE: u8 = 73;
    pub const GET_KEYBOARD_MAPPING: u8 = 101;
}

/// The predefined atoms.
const WM_NAME: u32 = 39;
const WM_NORMAL_HINTS: u32 = 40;
const WM_SIZE_HINTS: u32 = 41;
const STRING: u32 = 31;
const ATOM: u32 = 4;

/// The events of the window: key press and release, button press and release, pointer motion,
/// exposure, structure notify and focus change.
const EVENT_MASK: u32 = 0x1 | 0x2 | 0x4 | 0x8 | 0x40 | 0x8000 | 0x2_0000 | 0x20_0000;

/// How long a key release waits for the press of an auto-repeat.
const REPEAT_WAIT: Duration = Duration::from_millis(2);

/// The display of `DISPLAY`: a Unix socket for `:N` and `unix:N`, or TCP port 6000+N on a
/// host.
#[derive(Debug, PartialEq, Eq)]
enum Display {
    /// The path of the socket and the number of the display.
    Unix(String, String),
    /// The host, the port and the number of the display.
    Tcp(String, u16, String),
}

/// Read the value of `DISPLAY`, such as `:0`, `:1.0`, `unix:0` or `host:10.0`.
#[must_use]
fn display(v: &str) -> Option<Display> {
    let (host, rest) = v.rsplit_once(':')?;
    let number = rest.split('.').next()?;
    let n: u16 = number.parse().ok()?;
    if host.is_empty() || host == "unix" {
        return Some(Display::Unix(
            format!("/tmp/.X11-unix/X{n}"),
            number.to_string(),
        ));
    }
    Some(Display::Tcp(
        host.to_string(),
        6000u16.checked_add(n)?,
        number.to_string(),
    ))
}

/// The name and the data of the `MIT-MAGIC-COOKIE-1` entry for the display `number` in the
/// bytes of an `.Xauthority` file. An entry of family Local (256) or Wild (65535) matches. An
/// entry with no number matches each display.
#[must_use]
fn cookie(file: &[u8], number: &str) -> Option<Vec<u8>> {
    let mut p = 0;
    let field = |p: &mut usize| -> Option<Vec<u8>> {
        let n = usize::from(u16::from_be_bytes(file.get(*p..*p + 2)?.try_into().ok()?));
        let v = file.get(*p + 2..*p + 2 + n)?.to_vec();
        *p += 2 + n;
        Some(v)
    };
    while p + 2 <= file.len() {
        let family = u16::from_be_bytes(file[p..p + 2].try_into().ok()?);
        p += 2;
        let _address = field(&mut p)?;
        let num = field(&mut p)?;
        let name = field(&mut p)?;
        let data = field(&mut p)?;
        let local = family == 256 || family == 65535;
        if local && (num.is_empty() || num == number.as_bytes()) && name == b"MIT-MAGIC-COOKIE-1" {
            return Some(data);
        }
    }
    None
}

/// The pad that makes `n` bytes a multiple of 4.
const fn pad(n: usize) -> usize {
    (4 - n % 4) % 4
}

/// A request of the core protocol. The length goes in when the request ends.
struct Req(Vec<u8>);

impl Req {
    fn new(opcode: u8, data: u8) -> Self {
        Self(vec![opcode, data, 0, 0])
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn u16(mut self, v: u16) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u32(mut self, v: u32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn bytes(mut self, v: &[u8]) -> Self {
        self.0.extend_from_slice(v);
        self.0.resize(self.0.len() + pad(v.len()), 0);
        self
    }
    /// The bytes of the request, with its length in units of 4 bytes.
    fn end(mut self) -> Vec<u8> {
        let n = u16::try_from(self.0.len() / 4).unwrap_or(u16::MAX);
        self.0[2..4].copy_from_slice(&n.to_le_bytes());
        self.0
    }
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn i16_at(b: &[u8], at: usize) -> i16 {
    i16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// What the window needs from the reply of the connection setup.
#[derive(Debug, PartialEq, Eq)]
struct Setup {
    /// The base and the mask of the resource IDs of the client.
    id_base: u32,
    id_mask: u32,
    /// The longest request, in units of 4 bytes.
    max_request: u16,
    min_keycode: u8,
    max_keycode: u8,
    /// The root window of the first screen.
    root: u32,
    black: u32,
    root_depth: u8,
}

/// Read the additional data of a setup reply that succeeded. It is an error when the first
/// screen has no 24-bit `TrueColor` root visual with the masks of `x8r8g8b8`, when the depth 24
/// has no 32-bit pixels, or when the pixels are not little-endian.
fn setup(buf: &[u8]) -> Result<Setup, String> {
    let short = || "the setup reply of the X server is too short".to_string();
    if buf.len() < 32 {
        return Err(short());
    }
    let vendor = usize::from(u16_at(buf, 16));
    let max_request = u16_at(buf, 18);
    let nformats = usize::from(buf[21]);
    if buf[22] != 0 {
        return Err("the X server has big-endian images".into());
    }
    let (min_keycode, max_keycode) = (buf[26], buf[27]);
    let mut p = 32 + vendor + pad(vendor);
    let mut bpp24 = None;
    for _ in 0..nformats {
        let fmt = buf.get(p..p + 8).ok_or_else(short)?;
        if fmt[0] == 24 {
            bpp24 = Some(fmt[1]);
        }
        p += 8;
    }
    if bpp24 != Some(32) {
        return Err("the X server has no depth 24 with 32-bit pixels".into());
    }
    let scr = buf.get(p..p + 40).ok_or_else(short)?;
    let root = u32_at(scr, 0);
    let black = u32_at(scr, 12);
    let root_visual = u32_at(scr, 32);
    let root_depth = scr[38];
    let ndepths = scr[39];
    p += 40;
    let mut good = false;
    for _ in 0..ndepths {
        let dep = buf.get(p..p + 8).ok_or_else(short)?;
        let (depth, nvisuals) = (dep[0], usize::from(u16_at(dep, 2)));
        p += 8;
        for _ in 0..nvisuals {
            let vis = buf.get(p..p + 24).ok_or_else(short)?;
            if u32_at(vis, 0) == root_visual
                && depth == 24
                && vis[4] == 4
                && (u32_at(vis, 8), u32_at(vis, 12), u32_at(vis, 16)) == (0xff_0000, 0xff00, 0xff)
            {
                good = true;
            }
            p += 24;
        }
    }
    if !good || root_depth != 24 {
        return Err("the root visual of the X server is not 24-bit TrueColor x8r8g8b8".into());
    }
    Ok(Setup {
        id_base: u32_at(buf, 4),
        id_mask: u32_at(buf, 8),
        max_request,
        min_keycode,
        max_keycode,
        root,
        black,
        root_depth,
    })
}

/// The keysyms of each keycode, from `GetKeyboardMapping`.
#[derive(Debug, Default)]
struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

/// The modifier bits of the state of an event.
const SHIFT: u16 = 1;
const LOCK: u16 = 2;
const CONTROL: u16 = 4;
/// The button bits of the state of an event.
const BUTTONS: u16 = 0x700;

impl Keymap {
    /// The keysym of `keycode` with the modifiers of `state`. With Shift, or with Caps Lock
    /// and a letter, it is the second keysym of the key when the key has one.
    #[must_use]
    fn keysym(&self, keycode: u8, state: u16) -> u32 {
        let Some(i) = keycode.checked_sub(self.min) else {
            return 0;
        };
        let at = usize::from(i) * self.per;
        let col = |c: usize| {
            if c < self.per {
                self.syms.get(at + c).copied().unwrap_or(0)
            } else {
                0
            }
        };
        let (lower, upper) = (col(0), col(1));
        let letter = (0x61..=0x7a).contains(&lower);
        let shifted = state & SHIFT != 0 || (state & LOCK != 0 && letter);
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

/// The rune of `keyboard.h` for `keysym` with the modifiers of `state`, or None for a key that
/// gives nothing, such as Shift.
#[must_use]
fn keysym_rune(keysym: u32, state: u16) -> Option<u32> {
    let r = match keysym {
        0x20..=0x7e | 0xa0..=0xff => keysym,
        0x0100_0000..=0x0110_ffff => keysym - 0x0100_0000,
        0xff08 => 0x08,
        0xff09 => 0x09,
        0xff0d | 0xff8d => u32::from(b'\n'),
        0xff1b => 0x1b,
        0xffff => k::DEL,
        0xff50 => k::HOME,
        0xff51 => k::LEFT,
        0xff52 => k::UP,
        0xff53 => k::RIGHT,
        0xff54 => k::DOWN,
        0xff55 => k::PGUP,
        0xff56 => k::PGDOWN,
        0xff57 => k::END,
        0xff61 => k::PRINT,
        0xff63 => k::INS,
        0xffb0..=0xffb9 => u32::from(b'0') + keysym - 0xffb0,
        0xffbe..=0xffc9 => k::KF + 1 + keysym - 0xffbe,
        0xffe1 | 0xffe2 => k::SHIFT,
        0xffe3 | 0xffe4 => k::CTL,
        0xffe9 | 0xffea => k::ALT,
        0xfe03 | 0xff7e => k::ALTGR,
        0xffeb | 0xffec => k::MOD4,
        _ => return None,
    };
    if state & CONTROL != 0 && (0x40..=0x7f).contains(&(r & !0x20)) && r != k::DEL {
        return Some(r & 0x1f);
    }
    Some(r)
}

/// The buttons of `/dev/mouse` for the buttons of the state of an X event.
#[must_use]
const fn state_buttons(state: u16) -> u8 {
    ((state & BUTTONS) >> 8) as u8
}

/// The bit of `/dev/mouse` for the X button `b`: 1, 2 and 3 are 1, 2 and 4, and the wheel up
/// and down (4 and 5) are 8 and 16.
#[must_use]
const fn button_bit(b: u8) -> u8 {
    match b {
        1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        _ => 0,
    }
}

/// An event from the X server that the window uses.
#[derive(Debug, PartialEq, Eq)]
enum Event {
    /// A key press (true) or release (false), with the keycode, the state and the time.
    Key(bool, u8, u16, u32),
    FocusOut,
    Button(bool, u8, i16, i16),
    Motion(i16, i16, u16),
    Expose,
    Close(u32),
    Error(u8, u8),
    /// The connection ended.
    Gone,
}

/// The event of a packet of 32 bytes, or None for an event that the window does not use.
#[must_use]
fn event(b: &[u8; 32]) -> Option<Event> {
    Some(match b[0] & 0x7f {
        0 => Event::Error(b[1], b[10]),
        2 | 3 => Event::Key(b[0] & 0x7f == 2, b[1], u16_at(b, 28), u32_at(b, 4)),
        4 | 5 => Event::Button(b[0] & 0x7f == 4, b[1], i16_at(b, 24), i16_at(b, 26)),
        6 => Event::Motion(i16_at(b, 24), i16_at(b, 26), u16_at(b, 28)),
        10 => Event::FocusOut,
        12 if u16_at(b, 16) == 0 => Event::Expose,
        33 if b[1] == 32 => Event::Close(u32_at(b, 12)),
        _ => return None,
    })
}

/// The stream to the X server.
trait Stream: Read + Write + Send {
    fn reader(&self) -> std::io::Result<Box<dyn Read + Send>>;
}

impl Stream for UnixStream {
    fn reader(&self) -> std::io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.try_clone()?))
    }
}

impl Stream for TcpStream {
    fn reader(&self) -> std::io::Result<Box<dyn Read + Send>> {
        Ok(Box::new(self.try_clone()?))
    }
}

/// A connection to the X server.
struct Conn {
    s: Box<dyn Stream>,
    setup: Setup,
    next_id: u32,
}

/// Read a reply to the request that was sent last. Events that come first are dropped, and an
/// error is an `Err`.
fn reply<R: Read + ?Sized>(stream: &mut R) -> Result<Vec<u8>, String> {
    let io = |io: std::io::Error| format!("the X server: {io}");
    loop {
        let mut head = [0u8; 32];
        stream.read_exact(&mut head).map_err(io)?;
        match head[0] {
            0 => {
                return Err(format!(
                    "the X server gave error {} for request {}",
                    head[1], head[10]
                ));
            }
            1 => {
                let len = u32_at(&head, 4) as usize * 4;
                let mut body = head.to_vec();
                body.resize(32 + len, 0);
                stream.read_exact(&mut body[32..]).map_err(io)?;
                return Ok(body);
            }
            35 => {
                let mut rest = vec![0u8; u32_at(&head, 4) as usize * 4];
                stream.read_exact(&mut rest).map_err(io)?;
            }
            _ => {}
        }
    }
}

impl Conn {
    /// Connect to the display of `DISPLAY`, with the cookie of `XAUTHORITY` or
    /// `~/.Xauthority` when there is one.
    fn open() -> Result<Self, String> {
        let var = std::env::var("DISPLAY").map_err(|_| "DISPLAY is not set".to_string())?;
        let disp = display(&var).ok_or_else(|| format!("DISPLAY={var}: the value is not known"))?;
        let (mut stream, number): (Box<dyn Stream>, String) = match disp {
            Display::Unix(path, num) => (
                Box::new(UnixStream::connect(&path).map_err(|io| format!("{path}: {io}"))?),
                num,
            ),
            Display::Tcp(host, port, num) => (
                Box::new(
                    TcpStream::connect((host.as_str(), port))
                        .map_err(|io| format!("{host}:{port}: {io}"))?,
                ),
                num,
            ),
        };
        let auth = std::env::var_os("XAUTHORITY")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".Xauthority"))
            })
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|f| cookie(&f, &number));
        let (name, data): (&[u8], Vec<u8>) = match &auth {
            Some(c) => (b"MIT-MAGIC-COOKIE-1", c.clone()),
            None => (b"", Vec::new()),
        };
        let mut hello = vec![b'l', 0, 11, 0, 0, 0];
        hello.extend_from_slice(&u16::try_from(name.len()).unwrap_or(0).to_le_bytes());
        hello.extend_from_slice(&u16::try_from(data.len()).unwrap_or(0).to_le_bytes());
        hello.extend_from_slice(&[0, 0]);
        for var in [name, &data[..]] {
            hello.extend_from_slice(var);
            hello.resize(hello.len() + pad(var.len()), 0);
        }
        let io = |io: std::io::Error| format!("the X server: {io}");
        stream.write_all(&hello).map_err(io)?;
        let mut head = [0u8; 8];
        stream.read_exact(&mut head).map_err(io)?;
        let mut more = vec![0u8; usize::from(u16_at(&head, 6)) * 4];
        stream.read_exact(&mut more).map_err(io)?;
        if head[0] != 1 {
            let num = if head[0] == 0 {
                usize::from(head[1])
            } else {
                more.len()
            };
            let why = String::from_utf8_lossy(&more[..num.min(more.len())]);
            return Err(format!(
                "the X server refused the connection: {}",
                why.trim_end_matches('\0')
            ));
        }
        let setup = setup(&more)?;
        Ok(Self {
            s: stream,
            setup,
            next_id: 1,
        })
    }

    /// A new resource ID.
    fn id(&mut self) -> u32 {
        let id = self.setup.id_base | (self.next_id & self.setup.id_mask);
        self.next_id += 1;
        id
    }

    fn send(&mut self, r: &[u8]) -> Result<(), String> {
        self.s
            .write_all(r)
            .map_err(|e| format!("the X server: {e}"))
    }

    fn call(&mut self, r: &[u8]) -> Result<Vec<u8>, String> {
        self.send(r)?;
        reply(&mut *self.s)
    }

    fn atom(&mut self, name: &str) -> Result<u32, String> {
        let r = Req::new(op::INTERN_ATOM, 0)
            .u16(u16::try_from(name.len()).unwrap_or(0))
            .u16(0)
            .bytes(name.as_bytes())
            .end();
        Ok(u32_at(&self.call(&r)?, 8))
    }

    fn keymap(&mut self) -> Result<Keymap, String> {
        let (min, max) = (self.setup.min_keycode, self.setup.max_keycode);
        let r = Req::new(op::GET_KEYBOARD_MAPPING, 0)
            .u8(min)
            .u8(max - min + 1)
            .u16(0)
            .end();
        let b = self.call(&r)?;
        let syms = b[32..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c))
            .collect();
        Ok(Keymap {
            min,
            per: usize::from(b[1]),
            syms,
        })
    }

    /// Make a window of `w` by `h` pixels that the window manager does not resize, and its
    /// graphics context. Give the IDs of the window and the context.
    fn window(&mut self, title: &str, w: u16, h: u16) -> Result<(u32, u32), String> {
        let (win, gc) = (self.id(), self.id());
        let root = self.setup.root;
        let black = self.setup.black;
        let r = Req::new(op::CREATE_WINDOW, 0)
            .u32(win)
            .u32(root)
            .u16(0)
            .u16(0)
            .u16(w)
            .u16(h)
            .u16(0)
            .u16(1)
            .u32(0)
            .u32(0x2 | 0x800)
            .u32(black)
            .u32(EVENT_MASK)
            .end();
        self.send(&r)?;
        self.property(win, WM_NAME, STRING, 8, title.as_bytes(), title.len())?;
        let mut hints = [0u32; 18];
        hints[0] = 16 | 32;
        hints[5] = u32::from(w);
        hints[6] = u32::from(h);
        hints[7] = u32::from(w);
        hints[8] = u32::from(h);
        let bytes: Vec<u8> = hints.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.property(win, WM_NORMAL_HINTS, WM_SIZE_HINTS, 32, &bytes, hints.len())?;
        let r = Req::new(op::CREATE_GC, 0).u32(gc).u32(win).u32(0).end();
        self.send(&r)?;
        Ok((win, gc))
    }

    /// Set the property `prop` of `win` to `data`, which has `n` items of `format` bits.
    fn property(
        &mut self,
        win: u32,
        prop: u32,
        kind: u32,
        format: u8,
        data: &[u8],
        n: usize,
    ) -> Result<(), String> {
        let r = Req::new(op::CHANGE_PROPERTY, 0)
            .u32(win)
            .u32(prop)
            .u32(kind)
            .u8(format)
            .u8(0)
            .u16(0)
            .u32(u32::try_from(n).unwrap_or(0))
            .bytes(data)
            .end();
        self.send(&r)
    }

    fn map(&mut self, win: u32) -> Result<(), String> {
        let r = Req::new(op::MAP_WINDOW, 0).u32(win).end();
        self.send(&r)
    }

    fn destroy(&mut self, win: u32) -> Result<(), String> {
        let r = Req::new(op::DESTROY_WINDOW, 0).u32(win).end();
        self.send(&r)
    }

    /// Draw the screen of `w` by `h` pixels in the window, in strips of rows that each fit in
    /// one request.
    fn put(
        &mut self,
        win: u32,
        gc: u32,
        width: u32,
        height: u32,
        pixels: &[u32],
    ) -> Result<(), String> {
        let (cols, lines) = (width as usize, height as usize);
        if cols == 0 || pixels.len() < cols * lines {
            return Ok(());
        }
        let room = usize::from(self.setup.max_request).saturating_sub(6) * 4;
        let rows = (room / (cols * 4)).max(1);
        let mut out = Vec::new();
        let mut row = 0;
        while row < lines {
            let count = rows.min(lines - row);
            let mut req = Req::new(op::PUT_IMAGE, 2)
                .u32(win)
                .u32(gc)
                .u16(u16::try_from(width).unwrap_or(0))
                .u16(u16::try_from(count).unwrap_or(0))
                .u16(0)
                .u16(u16::try_from(row).unwrap_or(0))
                .u8(0)
                .u8(24)
                .u16(0);
            req.0.reserve(cols * count * 4);
            for px in &pixels[row * cols..(row + count) * cols] {
                req.0.extend_from_slice(&px.to_le_bytes());
            }
            out.extend_from_slice(&req.end());
            row += count;
        }
        self.send(&out)
    }

    /// The pixels of the rectangle of `win` at (0, 0) of `w` by `h` pixels.
    #[cfg(test)]
    fn get(&mut self, win: u32, w: u16, h: u16) -> Result<Vec<u32>, String> {
        let r = Req::new(op::GET_IMAGE, 2)
            .u32(win)
            .u16(0)
            .u16(0)
            .u16(w)
            .u16(h)
            .u32(u32::MAX)
            .end();
        let b = self.call(&r)?;
        Ok(b[32..]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|c| u32::from_le_bytes(*c) & 0x00ff_ffff)
            .collect())
    }

    /// A thread that reads the events of the connection and sends them to the channel `tx`.
    /// At the end of the connection, it sends `Gone`.
    fn events(&self, tx: mpsc::Sender<Event>) -> Result<(), String> {
        let mut r = self.s.reader().map_err(|e| format!("the X server: {e}"))?;
        std::thread::spawn(move || {
            let mut b = [0u8; 32];
            while r.read_exact(&mut b).is_ok() {
                if b[0] == 1 || b[0] == 35 {
                    let mut rest = vec![0u8; u32_at(&b, 4) as usize * 4];
                    if r.read_exact(&mut rest).is_err() {
                        break;
                    }
                    continue;
                }
                if let Some(e) = event(&b)
                    && tx.send(e).is_err()
                {
                    return;
                }
            }
            let _ = tx.send(Event::Gone);
        });
        Ok(())
    }
}

/// The input state of the window.
struct Input {
    keymap: Keymap,
    held: Held,
    buttons: u8,
    w: u32,
    h: u32,
}

impl Input {
    /// The rune of the key `code` with the modifiers of `state`, or 0 for a key that gives none.
    fn rune(&self, code: u8, state: u16) -> u32 {
        keysym_rune(self.keymap.keysym(code, state), state).unwrap_or(0)
    }

    fn point(&self, x: i16, y: i16) -> (u16, u16) {
        let c = |v: i16, n: u32| {
            u16::try_from(v.max(0))
                .unwrap_or(0)
                .min(u16::try_from(n.saturating_sub(1)).unwrap_or(u16::MAX))
        };
        (c(x, self.w), c(y, self.h))
    }
}

/// Give the key events to the host.
fn give(host: &dyn Host, events: Vec<crate::KeyEvent>) {
    for (r, down) in events {
        host.key(r, down);
    }
}

/// Show a window of `width` by `height` pixels with the title `title`, and run the event loop
/// until the session ends.
pub fn run(title: &str, width: u32, height: u32, host: &dyn Host) -> Result<(), String> {
    let mut c = Conn::open()?;
    let keymap = c.keymap()?;
    let protocols = c.atom("WM_PROTOCOLS")?;
    let delete = c.atom("WM_DELETE_WINDOW")?;
    let size = |v: u32| u16::try_from(v).map_err(|_| format!("the screen size {v} is too large"));
    let (win, gc) = c.window(title, size(width)?, size(height)?)?;
    c.property(win, protocols, ATOM, 32, &delete.to_le_bytes(), 1)?;
    c.map(win)?;
    let (tx, rx) = mpsc::channel();
    c.events(tx)?;
    let mut input = Input {
        keymap,
        held: Held::default(),
        buttons: 0,
        w: width,
        h: height,
    };
    let mut generation = 0u64;
    let mut pixels = Vec::new();
    let mut shown: Option<(u32, u32)> = None;
    let mut next = Instant::now();
    // An event that came while a key release waited for the press of an auto-repeat.
    let mut early: Option<Event> = None;
    loop {
        if host.done() {
            let _ = c.destroy(win);
            return Ok(());
        }
        // A frame is due even when the events do not stop, as in a drag of the mouse.
        let now = Instant::now();
        if now >= next {
            if let Some((w, h)) = host.frame(&mut generation, &mut pixels) {
                shown = Some((w, h));
                input.w = w;
                input.h = h;
                c.put(win, gc, w, h, &pixels)?;
            }
            next += FRAME;
            if next < now {
                next = now + FRAME;
            }
        }
        let ev = match early.take() {
            Some(ev) => Ok(ev),
            None => rx.recv_timeout(next.saturating_duration_since(Instant::now())),
        };
        match ev {
            Ok(Event::Key(true, code, state, _)) => {
                let r = input.rune(code, state);
                give(host, input.held.press(u32::from(code), r));
            }
            Ok(Event::Key(false, code, state, time)) => match rx.recv_timeout(REPEAT_WAIT) {
                // The auto-repeat: the press that comes next is a key down of the same rune.
                Ok(Event::Key(true, c, _, t)) if c == code && t == time => {
                    let r = input.rune(code, state);
                    give(host, input.held.press(u32::from(code), r));
                }
                next => {
                    early = next.ok();
                    give(host, input.held.release(u32::from(code)));
                }
            },
            Ok(Event::FocusOut) => give(host, input.held.release_all()),
            Ok(Event::Button(press, b, x, y)) => {
                let (x, y) = input.point(x, y);
                let bit = button_bit(b);
                if bit >= 8 {
                    if press {
                        host.mouse(x, y, input.buttons | bit);
                        host.mouse(x, y, input.buttons);
                    }
                } else {
                    if press {
                        input.buttons |= bit;
                    } else {
                        input.buttons &= !bit;
                    }
                    host.mouse(x, y, input.buttons);
                }
            }
            Ok(Event::Motion(x, y, state)) => {
                let (x, y) = input.point(x, y);
                input.buttons = state_buttons(state);
                host.mouse(x, y, input.buttons);
            }
            Ok(Event::Expose) => {
                if let Some((w, h)) = shown {
                    c.put(win, gc, w, h, &pixels)?;
                }
            }
            Ok(Event::Close(atom)) if atom == delete => host.quit(),
            Ok(Event::Error(code, request)) => {
                eprintln!("9h: the X server gave error {code} for request {request}");
            }
            Ok(Event::Gone) | Err(RecvTimeoutError::Disconnected) => {
                host.quit();
                return Err("the X server closed the connection".into());
            }
            Ok(Event::Close(_)) | Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_display_names_the_socket() {
        assert_eq!(
            display(":0"),
            Some(Display::Unix("/tmp/.X11-unix/X0".into(), "0".into()))
        );
        assert_eq!(
            display("unix:12.0"),
            Some(Display::Unix("/tmp/.X11-unix/X12".into(), "12".into()))
        );
        assert_eq!(
            display("localhost:10.0"),
            Some(Display::Tcp("localhost".into(), 6010, "10".into()))
        );
        assert_eq!(display("nothing"), None);
        assert_eq!(display(":x"), None);
    }

    fn entry(family: u16, number: &str, name: &str, data: &[u8]) -> Vec<u8> {
        let mut v = family.to_be_bytes().to_vec();
        for f in [b"host".as_slice(), number.as_bytes(), name.as_bytes(), data] {
            v.extend_from_slice(&u16::try_from(f.len()).unwrap().to_be_bytes());
            v.extend_from_slice(f);
        }
        v
    }

    #[test]
    fn the_cookie_is_that_of_the_display() {
        let mut f = entry(0, "0", "MIT-MAGIC-COOKIE-1", b"tcp");
        f.extend(entry(256, "1", "MIT-MAGIC-COOKIE-1", b"one"));
        f.extend(entry(256, "0", "XDM-AUTHORIZATION-1", b"xdm"));
        f.extend(entry(256, "0", "MIT-MAGIC-COOKIE-1", b"zero"));
        assert_eq!(cookie(&f, "0"), Some(b"zero".to_vec()));
        assert_eq!(cookie(&f, "1"), Some(b"one".to_vec()));
        assert_eq!(cookie(&f, "2"), None);
        assert_eq!(cookie(&f[..5], "0"), None);
        let wild = entry(65535, "", "MIT-MAGIC-COOKIE-1", b"any");
        assert_eq!(cookie(&wild, "7"), Some(b"any".to_vec()));
    }

    #[test]
    fn a_request_has_its_length_and_its_pad() {
        let r = Req::new(op::INTERN_ATOM, 0)
            .u16(5)
            .u16(0)
            .bytes(b"HELLO")
            .end();
        assert_eq!(r.len(), 16);
        assert_eq!(&r[..4], &[16, 0, 4, 0]);
        assert_eq!(&r[8..16], b"HELLO\0\0\0");
    }

    /// A setup reply with one format and one screen with one visual.
    fn reply_bytes(mask: u32, bpp: u8) -> Vec<u8> {
        let mut b = vec![0u8; 32];
        b[4..8].copy_from_slice(&0x0040_0000u32.to_le_bytes());
        b[8..12].copy_from_slice(&0x001f_ffffu32.to_le_bytes());
        b[16..18].copy_from_slice(&3u16.to_le_bytes());
        b[18..20].copy_from_slice(&65535u16.to_le_bytes());
        b[20] = 1;
        b[21] = 1;
        b[26] = 8;
        b[27] = 255;
        b.extend_from_slice(b"Xyz\0");
        b.extend_from_slice(&[24, bpp, 32, 0, 0, 0, 0, 0]);
        let mut s = vec![0u8; 40];
        s[0..4].copy_from_slice(&0x1e2u32.to_le_bytes());
        s[32..36].copy_from_slice(&0x21u32.to_le_bytes());
        s[38] = 24;
        s[39] = 1;
        b.extend_from_slice(&s);
        b.extend_from_slice(&[24, 0, 1, 0, 0, 0, 0, 0]);
        let mut v = vec![0u8; 24];
        v[0..4].copy_from_slice(&0x21u32.to_le_bytes());
        v[4] = 4;
        v[8..12].copy_from_slice(&mask.to_le_bytes());
        v[12..16].copy_from_slice(&0xff00u32.to_le_bytes());
        v[16..20].copy_from_slice(&0xffu32.to_le_bytes());
        b.extend_from_slice(&v);
        b
    }

    #[test]
    fn the_setup_gives_the_root_and_the_ids() {
        let s = setup(&reply_bytes(0xff_0000, 32)).unwrap();
        assert_eq!(s.root, 0x1e2);
        assert_eq!(s.id_base, 0x0040_0000);
        assert_eq!((s.min_keycode, s.max_keycode), (8, 255));
        assert_eq!(s.max_request, 65535);
        assert!(setup(&reply_bytes(0xff, 32)).is_err());
        assert!(setup(&reply_bytes(0xff_0000, 24)).is_err());
        assert!(setup(&reply_bytes(0xff_0000, 32)[..60]).is_err());
    }

    #[test]
    fn the_keys_give_the_runes_of_plan_9() {
        let m = Keymap {
            min: 8,
            per: 2,
            syms: vec![0x61, 0x41, 0x31, 0x21, 0xff0d, 0, 0xff52, 0],
        };
        assert_eq!(m.keysym(8, 0), 0x61);
        assert_eq!(m.keysym(8, SHIFT), 0x41);
        assert_eq!(m.keysym(8, LOCK), 0x41);
        assert_eq!(m.keysym(9, LOCK), 0x31);
        assert_eq!(m.keysym(9, SHIFT), 0x21);
        assert_eq!(m.keysym(10, SHIFT), 0xff0d);
        assert_eq!(m.keysym(7, 0), 0);
        assert_eq!(m.keysym(200, 0), 0);
        assert_eq!(keysym_rune(0x61, 0), Some(0x61));
        assert_eq!(keysym_rune(0x61, CONTROL), Some(1));
        assert_eq!(keysym_rune(0x5b, CONTROL), Some(0x1b));
        assert_eq!(keysym_rune(0xff0d, 0), Some(0x0a));
        assert_eq!(keysym_rune(0xff08, 0), Some(0x08));
        assert_eq!(keysym_rune(0xffff, CONTROL), Some(0x7f));
        assert_eq!(keysym_rune(0xff52, 0), Some(0xF00E));
        assert_eq!(keysym_rune(0xff54, 0), Some(0x80));
        assert_eq!(keysym_rune(0xffbe, 0), Some(0xF001));
        assert_eq!(keysym_rune(0x0100_00e9, 0), Some(0xe9));
        assert_eq!(keysym_rune(0xffe1, 0), Some(k::SHIFT));
        assert_eq!(keysym_rune(0xffe4, CONTROL), Some(k::CTL));
        assert_eq!(keysym_rune(0xffe9, 0), Some(k::ALT));
        assert_eq!(keysym_rune(0xfe03, 0), Some(k::ALTGR));
        assert_eq!(keysym_rune(0xffeb, 0), Some(k::MOD4));
        assert_eq!(keysym_rune(0xffe5, 0), None);
    }

    #[test]
    fn the_buttons_and_the_events_are_those_of_plan_9() {
        assert_eq!(state_buttons(0x100), 1);
        assert_eq!(state_buttons(0x200), 2);
        assert_eq!(state_buttons(0x400), 4);
        assert_eq!(state_buttons(0x700 | SHIFT), 7);
        assert_eq!(button_bit(3), 4);
        assert_eq!(button_bit(5), 16);
        assert_eq!(button_bit(9), 0);
        let mut b = [0u8; 32];
        b[0] = 4;
        b[1] = 3;
        b[24..26].copy_from_slice(&10i16.to_le_bytes());
        b[26..28].copy_from_slice(&(-2i16).to_le_bytes());
        assert_eq!(event(&b), Some(Event::Button(true, 3, 10, -2)));
        b[0] = 0x80 | 2;
        b[1] = 38;
        b[4..8].copy_from_slice(&9u32.to_le_bytes());
        b[28] = 1;
        assert_eq!(event(&b), Some(Event::Key(true, 38, 1, 9)));
        b[0] = 3;
        assert_eq!(event(&b), Some(Event::Key(false, 38, 1, 9)));
        b = [0u8; 32];
        b[0] = 10;
        assert_eq!(event(&b), Some(Event::FocusOut));
        b = [0u8; 32];
        b[0] = 12;
        b[16] = 1;
        assert_eq!(event(&b), None);
        b[16] = 0;
        assert_eq!(event(&b), Some(Event::Expose));
        b = [0u8; 32];
        b[0] = 33;
        b[1] = 32;
        b[12..16].copy_from_slice(&77u32.to_le_bytes());
        assert_eq!(event(&b), Some(Event::Close(77)));
    }

    /// With `H9_NEED_X11` set, the test needs an X server on `DISPLAY`, such as Xvfb in CI. It
    /// draws an image in a window, and reads the pixels back. With no `H9_NEED_X11` and no
    /// `DISPLAY`, the test does nothing.
    #[test]
    fn a_window_shows_the_pixels_of_the_screen() {
        let need = std::env::var_os("H9_NEED_X11").is_some();
        if !need && std::env::var_os("DISPLAY").is_none() {
            return;
        }
        let mut c = Conn::open().unwrap();
        let (w, h) = (300u16, 200u16);
        let (win, gc) = c.window("h9-gui test", w, h).unwrap();
        c.map(win).unwrap();
        let pixels: Vec<u32> = (0..u32::from(w) * u32::from(h))
            .map(|i| i.wrapping_mul(2_654_435_761) & 0x00ff_ffff)
            .collect();
        // The window is on the screen some time after the map, and a draw before that is lost.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            c.put(win, gc, u32::from(w), u32::from(h), &pixels).unwrap();
            if c.get(win, w, h).unwrap() == pixels {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "the window never showed the pixels"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        c.destroy(win).unwrap();
    }
}
