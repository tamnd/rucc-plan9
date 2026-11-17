//! G/Q for `r9 gui run --on qemu`: the GUI scripts on a 9front kernel under QEMU (plan 12.6).
//!
//! `r9 gui run` gives the steps of a script to the control socket of the 9h monitor. This module
//! gives the same steps to QEMU, so that a script runs on a real 9front kernel with no change.
//! A thread of r9 listens on a Unix socket in the work directory and does each line as the 9h
//! monitor does:
//!
//! - `send TEXT` types each rune of TEXT with the QMP command `input-send-event` on the PS/2
//!   keyboard, with the US layout;
//! - `mouse X Y B` and `click X Y [B]` move the pointer of a USB tablet to the pixel X, Y and set
//!   the buttons, with the same command;
//! - `quiet` and `snap` read the screen with the QMP command `screendump`;
//! - `wait TEXT` and `absent TEXT` read `console.log`, the serial console;
//! - `regs` writes the registers of each CPU to `regs.txt`.
//!
//! QEMU does not tell when the guest reads an event, and the 9h monitor does. So each key and each
//! mouse event waits a fixed time after it, and a move that comes before the guest reads the move
//! before it can replace that move, as on real hardware.
//!
//! The kernel of 9front amd64 has no real mode, so the VESA mode is set by `9boot` from
//! `plan9.ini`. [`prepare`] boots the root image one time without a screen. It adds `monitor=vesa`
//! and `vgasize=1024x768x32` to `plan9.ini` and puts the data files of the scripts in
//! `/lib/r9data`. The overlay of that boot is the base of each run. The screen then has the size
//! of the screen of 9h, so a window of a script is at the same place on G/Q and on G/H. A script
//! that mounts `#Z/hostfs` binds `/lib/r9data` when there is no `#Z`.

use crate::boot::{self, Outcome};
use crate::gui::{self, Image};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The size of the screen that [`prepare`] sets, the size of the screen of 9h.
pub const SCREEN: (u32, u32) = gui::SCREEN;

/// The time after each key event. rio and samterm read the keys in a loop, and a burst of keys
/// loses keys.
const KEY_GAP: Duration = Duration::from_millis(40);

/// The time after each mouse event, longer than the poll interval of the USB tablet.
const MOUSE_GAP: Duration = Duration::from_millis(60);

/// The time between two screen dumps of a `quiet` step.
const LOOK_GAP: Duration = Duration::from_millis(50);

/// The highest value of an absolute axis of `input-send-event`.
const ABS_SPAN: u32 = 0x8000;

/// The data files of the scripts in the guest.
pub const DATA_DIR: &str = "/lib/r9data";

/// The lines of `plan9.ini` that set the screen.
fn vga_lines() -> [String; 2] {
    let (w, h) = SCREEN;
    ["monitor=vesa".into(), format!("vgasize={w}x{h}x32")]
}

/// The commands of the boot of [`prepare`]. `data` tells if the input disk has the data files.
#[must_use]
pub fn prepare_commands(data: bool) -> Vec<String> {
    let [m, v] = vga_lines();
    let mut c = vec![
        "9fs 9fat".to_string(),
        "grep -v '^(monitor|vgasize)=' /n/9fat/plan9.ini > /tmp/plan9.ini".into(),
        format!("{{cat /tmp/plan9.ini; echo '{m}'; echo '{v}'}} > /n/9fat/plan9.ini"),
        "cat /n/9fat/plan9.ini".into(),
        "unmount /n/9fat".into(),
    ];
    if data {
        c.push(format!(
            "mkdir -p {DATA_DIR} && cd {DATA_DIR} && tar xf $r9in && cd /"
        ));
    }
    c
}

/// A short hash of the root image and of the data files, which names the base of [`prepare`].
fn base_key(root: &Path, data: Option<&Path>) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    let meta = std::fs::metadata(root).map_err(|e| format!("{}: {e}", root.display()))?;
    h.update(root.display().to_string().as_bytes());
    h.update(meta.len().to_be_bytes());
    if let Ok(t) = meta.modified() {
        h.update(format!("{t:?}").as_bytes());
    }
    for line in vga_lines() {
        h.update(line.as_bytes());
    }
    if let Some(dir) = data {
        let mut files = Vec::new();
        walk(dir, &mut files)?;
        files.sort();
        for f in files {
            let rel = f.strip_prefix(dir).unwrap_or(&f);
            h.update(rel.display().to_string().as_bytes());
            h.update(std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?);
        }
    }
    Ok(h.finalize()[..6]
        .iter()
        .fold(String::new(), |s, b| s + &format!("{b:02x}")))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), String> {
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let p = e.map_err(|e| e.to_string())?.path();
        if p.is_dir() {
            walk(&p, out)?;
        } else {
            out.push(p);
        }
    }
    Ok(())
}

/// Make the base image of the runs of the root image of `req.base`, with the screen set and the
/// data files of `data`, in the directory `dir`, or give the base that is there already. The base
/// is an overlay of the root image, so the root image must stay where it is.
pub fn prepare(req: &boot::Request, data: Option<&Path>, dir: &Path) -> Result<PathBuf, String> {
    let key = base_key(&req.base, data)?;
    let base = dir.join(format!("base-{key}.qcow2"));
    if base.is_file() {
        return Ok(base);
    }
    let mut r = req.clone();
    r.work = dir.join(format!("prepare-{key}"));
    r.commands = prepare_commands(data.is_some());
    r.input = data.map(Path::to_path_buf);
    r.keep = true;
    r.timeout = Duration::from_mins(15);
    let outcome = boot::run(&r)?;
    if !outcome.passed {
        return Err(format!(
            "the boot that sets the screen did not pass; see {}",
            r.work.join("console.log").display()
        ));
    }
    std::fs::rename(boot::overlay(&r), &base).map_err(|e| format!("{}: {e}", base.display()))?;
    let _ = std::fs::remove_dir_all(&r.work);
    Ok(base)
}

/// The copy of the console that QEMU writes, for the `wait` and `absent` steps.
#[must_use]
pub fn serial_path(work: &Path) -> PathBuf {
    work.join("serial.log")
}

/// The QMP socket of a run.
#[must_use]
pub fn qmp_path(work: &Path) -> PathBuf {
    work.join("qmp.sock")
}

/// The control socket of a run, which takes the lines of the control socket of 9h.
#[must_use]
pub fn control_path(work: &Path) -> PathBuf {
    work.join("gq.sock")
}

/// The QEMU command of a run: the command of `r9 boot`, with a VGA card, a USB tablet and a QMP
/// socket.
#[must_use]
pub fn qemu_command(req: &boot::Request) -> Vec<String> {
    let mut c = boot::qemu_command(req);
    // The steps run while the console driver waits, and that driver writes `console.log`. Thus
    // QEMU writes a second copy of the console, which the `wait` and `absent` steps read.
    if let Some(i) = c
        .windows(2)
        .position(|w| w[0] == "-serial" && w[1] == "stdio")
    {
        c[i + 1] = "chardev:con".into();
        c.push("-chardev".into());
        c.push(format!(
            "stdio,id=con,logfile={},logappend=off",
            serial_path(&req.work).display()
        ));
    }
    for a in [
        "-vga",
        "std",
        "-device",
        "qemu-xhci",
        "-device",
        "usb-tablet",
        "-qmp",
    ] {
        c.push(a.into());
    }
    c.push(format!(
        "unix:{},server=on,wait=off",
        qmp_path(&req.work).display()
    ));
    c
}

/// A QMP connection.
pub struct Qmp {
    stream: std::os::unix::net::UnixStream,
    reader: BufReader<std::os::unix::net::UnixStream>,
}

impl Qmp {
    /// Connect to the QMP socket at `path`, and wait up to `limit` for QEMU to make it.
    pub fn connect(path: &Path, limit: Duration) -> Result<Self, String> {
        let end = Instant::now() + limit;
        let stream = loop {
            match std::os::unix::net::UnixStream::connect(path) {
                Ok(s) => break s,
                Err(e) if Instant::now() > end => return Err(format!("{}: {e}", path.display())),
                Err(_) => std::thread::sleep(Duration::from_millis(100)),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .map_err(|e| e.to_string())?;
        let reader = BufReader::new(stream.try_clone().map_err(|e| e.to_string())?);
        let mut q = Qmp { stream, reader };
        q.read()?;
        q.execute("qmp_capabilities", None)?;
        Ok(q)
    }

    fn read(&mut self) -> Result<Value, String> {
        let mut line = String::new();
        let n = self
            .reader
            .read_line(&mut line)
            .map_err(|e| format!("QMP: {e}"))?;
        if n == 0 {
            return Err("QMP: QEMU closed the socket".into());
        }
        serde_json::from_str(&line).map_err(|e| format!("QMP: {e}: {line}"))
    }

    /// Do the QMP command `cmd` with the arguments `args`, and give its return value. The events
    /// that come before the answer are dropped.
    pub fn execute(&mut self, cmd: &str, args: Option<Value>) -> Result<Value, String> {
        let msg = match args {
            Some(a) => json!({ "execute": cmd, "arguments": a }),
            None => json!({ "execute": cmd }),
        };
        writeln!(self.stream, "{msg}").map_err(|e| format!("QMP: {e}"))?;
        loop {
            let v = self.read()?;
            if let Some(r) = v.get("return") {
                return Ok(r.clone());
            }
            if let Some(e) = v.get("error") {
                return Err(format!("QMP {cmd}: {e}"));
            }
        }
    }
}

/// The modifier keys of a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    /// No modifier.
    None,
    /// Shift.
    Shift,
    /// Control.
    Ctrl,
}

/// The rune of a key of Plan 9 for the function keys and the arrows, from `keyboard.h`.
const KF: u32 = 0xF000;

/// The QEMU key code and the modifier that type the rune `r` on a US keyboard, or None.
#[must_use]
pub fn qcode(r: char) -> Option<(&'static str, Modifier)> {
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    const LETTERS: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    const FKEYS: [&str; 12] = [
        "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    ];
    // The punctuation of each key, without and with shift.
    const PUNCT: [(char, char, &str); 11] = [
        ('-', '_', "minus"),
        ('=', '+', "equal"),
        ('[', '{', "bracket_left"),
        (']', '}', "bracket_right"),
        ('\\', '|', "backslash"),
        (';', ':', "semicolon"),
        ('\'', '"', "apostrophe"),
        ('`', '~', "grave_accent"),
        (',', '<', "comma"),
        ('.', '>', "dot"),
        ('/', '?', "slash"),
    ];
    const SHIFTED_DIGITS: &str = ")!@#$%^&*(";
    use Modifier::{Ctrl, None as Plain, Shift};
    let u = u32::from(r);
    let index = |base: char| (u - u32::from(base)) as usize;
    Some(match r {
        'a'..='z' => (LETTERS[index('a')], Plain),
        'A'..='Z' => (LETTERS[index('A')], Shift),
        '0'..='9' => (DIGITS[index('0')], Plain),
        ' ' => ("spc", Plain),
        '\n' => ("ret", Plain),
        '\t' => ("tab", Plain),
        '\u{8}' => ("backspace", Plain),
        '\u{1b}' => ("esc", Plain),
        '\u{7f}' => ("delete", Plain),
        // Kdown is 0x80 in Plan 9.
        '\u{80}' => ("down", Plain),
        '\u{1}'..='\u{1a}' => (LETTERS[(u - 1) as usize], Ctrl),
        _ => {
            if let Some(i) = SHIFTED_DIGITS.find(r) {
                return Some((DIGITS[i], Shift));
            }
            if let Some(&(_, s, q)) = PUNCT.iter().find(|p| p.0 == r || p.1 == r) {
                return Some((q, if r == s { Shift } else { Plain }));
            }
            let code = match u.checked_sub(KF)? {
                n @ 1..=12 => FKEYS[(n - 1) as usize],
                0x0D => "home",
                0x0E => "up",
                0x0F => "pgup",
                0x10 => "print",
                0x11 => "left",
                0x12 => "right",
                0x13 => "pgdn",
                0x14 => "insert",
                0x18 => "end",
                _ => return None,
            };
            (code, Plain)
        }
    })
}

/// The value of an absolute axis for the pixel `p` of a screen `size` pixels wide. The USB
/// tablet driver of 9front gives the pixel `round(v * size / 0x8000)`.
#[must_use]
pub fn abs_value(p: u32, size: u32) -> u32 {
    ((p * ABS_SPAN + size / 2) / size).min(ABS_SPAN - 1)
}

/// The QMP buttons of the bits of a mouse line of 9h: 1 left, 2 middle, 4 right, 8 and 16 the
/// wheel.
const BUTTONS: [(u8, &str); 5] = [
    (1, "left"),
    (2, "middle"),
    (4, "right"),
    (8, "wheel-up"),
    (16, "wheel-down"),
];

/// Read a text argument: the rest of the line with no spaces at its ends, or a string in double
/// quotes with the escapes `\n`, `\r`, `\t`, `\\`, `\"` and `\xHH`.
pub fn text(arg: &str) -> Result<Vec<u8>, String> {
    let arg = arg.trim();
    let Some(inner) = arg.strip_prefix('"') else {
        return Ok(arg.as_bytes().to_vec());
    };
    let inner = inner
        .strip_suffix('"')
        .ok_or_else(|| format!("no end quote: {arg}"))?;
    let mut out = Vec::new();
    let mut b = inner.bytes();
    while let Some(c) = b.next() {
        if c != b'\\' {
            out.push(c);
            continue;
        }
        match b.next() {
            Some(b'n') => out.push(b'\n'),
            Some(b'r') => out.push(b'\r'),
            Some(b't') => out.push(b'\t'),
            Some(c @ (b'\\' | b'"')) => out.push(c),
            Some(b'x') => {
                let h: Vec<u8> = b.by_ref().take(2).collect();
                let s = std::str::from_utf8(&h).map_err(|e| e.to_string())?;
                out.push(u8::from_str_radix(s, 16).map_err(|_| format!("bad escape \\x{s}"))?);
            }
            c => return Err(format!("bad escape in {arg}: {c:?}")),
        }
    }
    Ok(out)
}

/// Read a portable pixmap (P6) with 8 bits for each color, as `screendump` writes it.
pub fn read_ppm(data: &[u8]) -> Result<Image, String> {
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while at < data.len() && data[at].is_ascii_whitespace() {
            at += 1;
        }
        if data.get(at) == Some(&b'#') {
            while at < data.len() && data[at] != b'\n' {
                at += 1;
            }
            continue;
        }
        let start = at;
        while at < data.len() && !data[at].is_ascii_whitespace() {
            at += 1;
        }
        if start == at {
            return Err("the PPM header is short".into());
        }
        fields.push(String::from_utf8_lossy(&data[start..at]).to_string());
    }
    at += 1;
    let num = |i: usize| -> Result<u32, String> {
        fields[i]
            .parse()
            .map_err(|_| format!("bad PPM header field {}", fields[i]))
    };
    if fields[0] != "P6" || num(3)? != 255 {
        return Err(format!("not a P6 PPM with 8-bit colors: {fields:?}"));
    }
    let (width, height) = (num(1)?, num(2)?);
    let n = width as usize * height as usize * 3;
    let rgb = data
        .get(at..at + n)
        .ok_or("the PPM pixels are short")?
        .to_vec();
    Ok(Image { width, height, rgb })
}

/// The rectangle `r` of the image `img`, or the whole image.
#[must_use]
pub fn crop(img: &Image, r: Option<[u32; 4]>) -> Image {
    let Some([x0, y0, x1, y1]) = r else {
        return img.clone();
    };
    let (x1, y1) = (x1.min(img.width), y1.min(img.height));
    let (x0, y0) = (x0.min(x1), y0.min(y1));
    let mut rgb = Vec::with_capacity(((x1 - x0) * (y1 - y0) * 3) as usize);
    for y in y0..y1 {
        let at = (y as usize * img.width as usize + x0 as usize) * 3;
        rgb.extend_from_slice(&img.rgb[at..at + (x1 - x0) as usize * 3]);
    }
    Image {
        width: x1 - x0,
        height: y1 - y0,
        rgb,
    }
}

/// True when `img` is a new rio window before its program draws: a border of 4 pixels, a white
/// inside and the gray line of the scroll bar at x = 16. The pointer can cover some pixels.
#[must_use]
pub fn blank_window(img: &Image) -> bool {
    const BORDER: u32 = 4;
    const SCROLL_X: u32 = 16;
    const WHITE: [u8; 3] = [255, 255, 255];
    const GRAY: [u8; 3] = [153, 153, 153];
    // The pixels of the pointer and of the text cursor.
    const OTHER: usize = 512;
    let (w, h) = (img.width, img.height);
    if w <= 2 * SCROLL_X || h <= 4 * BORDER {
        return false;
    }
    let (mut scroll, mut other) = (0, 0);
    for (i, p) in img.rgb.as_chunks::<3>().0.iter().enumerate() {
        let (x, y) = (i % w as usize, i / w as usize);
        let inside = (BORDER as usize..(w - BORDER) as usize).contains(&x)
            && (BORDER as usize..(h - BORDER) as usize).contains(&y);
        if !inside {
            continue;
        }
        if x == SCROLL_X as usize && *p == GRAY {
            scroll += 1;
        } else if *p != WHITE {
            other += 1;
        }
    }
    scroll * 10 >= (h as usize - 2 * BORDER as usize) * 9 && other <= OTHER
}

/// The machine that the lines drive: QEMU with its QMP socket and its console log.
pub struct Machine {
    qmp: Qmp,
    work: PathBuf,
    console: PathBuf,
    mark: usize,
    timeout: Duration,
    buttons: u8,
    at: (u32, u32),
}

/// The rectangle of the words `x0 y0 x1 y1`, or None for no words.
fn rect_of(words: &[&str]) -> Result<Option<[u32; 4]>, String> {
    if words.is_empty() {
        return Ok(None);
    }
    let n: Vec<u32> = words
        .iter()
        .map(|w| w.parse().map_err(|_| format!("bad number {w}")))
        .collect::<Result<_, _>>()?;
    match n[..] {
        [x0, y0, x1, y1] if x0 < x1 && y0 < y1 => Ok(Some([x0, y0, x1, y1])),
        _ => Err(format!("bad rectangle {}", words.join(" "))),
    }
}

fn seconds(w: &str) -> Result<Duration, String> {
    w.parse::<f64>()
        .ok()
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
        .ok_or_else(|| format!("bad number of seconds {w}"))
}

impl Machine {
    /// The machine of the run in `work`, with its QMP connection.
    #[must_use]
    pub fn new(qmp: Qmp, work: &Path) -> Self {
        Machine {
            qmp,
            work: work.to_path_buf(),
            console: serial_path(work),
            mark: 0,
            timeout: Duration::from_secs(60),
            buttons: 0,
            at: (0, 0),
        }
    }

    fn key(&mut self, code: &str, down: bool) -> Result<(), String> {
        let ev = json!({ "type": "key", "data": {
            "down": down, "key": { "type": "qcode", "data": code } } });
        self.qmp
            .execute("input-send-event", Some(json!({ "events": [ev] })))?;
        std::thread::sleep(KEY_GAP);
        Ok(())
    }

    /// Type the runes of `bytes`.
    fn send(&mut self, bytes: &[u8]) -> Result<(), String> {
        let s = std::str::from_utf8(bytes).map_err(|e| format!("the text is not UTF-8: {e}"))?;
        for r in s.chars() {
            let (code, m) =
                qcode(r).ok_or_else(|| format!("no key on a US keyboard types {r:?}"))?;
            let held = match m {
                Modifier::None => None,
                Modifier::Shift => Some("shift"),
                Modifier::Ctrl => Some("ctrl"),
            };
            if let Some(h) = held {
                self.key(h, true)?;
            }
            self.key(code, true)?;
            self.key(code, false)?;
            if let Some(h) = held {
                self.key(h, false)?;
            }
        }
        Ok(())
    }

    /// Move the pointer to (x, y) with the buttons `b` down.
    fn mouse(&mut self, x: u32, y: u32, b: u8) -> Result<(), String> {
        let (width, height) = SCREEN;
        if x >= width || y >= height {
            return Err(format!("{x},{y} is not on the {width}x{height} screen"));
        }
        let mut events = Vec::new();
        if (x, y) != self.at {
            events.push(
                json!({ "type": "abs", "data": { "axis": "x", "value": abs_value(x, width) } }),
            );
            events.push(
                json!({ "type": "abs", "data": { "axis": "y", "value": abs_value(y, height) } }),
            );
        }
        let wheel: Vec<&str> = BUTTONS
            .iter()
            .filter(|(bit, _)| *bit >= 8 && b & bit != 0)
            .map(|(_, n)| *n)
            .collect();
        for (bit, name) in BUTTONS.iter().filter(|(bit, _)| *bit < 8) {
            if (self.buttons ^ b) & bit != 0 {
                events.push(
                    json!({ "type": "btn", "data": { "down": b & bit != 0, "button": name } }),
                );
            }
        }
        if !events.is_empty() {
            self.qmp
                .execute("input-send-event", Some(json!({ "events": events })))?;
            std::thread::sleep(MOUSE_GAP);
        }
        for name in wheel {
            for down in [true, false] {
                let ev = json!({ "type": "btn", "data": { "down": down, "button": name } });
                self.qmp
                    .execute("input-send-event", Some(json!({ "events": [ev] })))?;
                std::thread::sleep(MOUSE_GAP);
            }
        }
        self.at = (x, y);
        self.buttons = b & 7;
        Ok(())
    }

    /// The screen now.
    fn screen(&mut self) -> Result<Image, String> {
        let f = self.work.join("screen.ppm");
        self.qmp.execute(
            "screendump",
            Some(json!({ "filename": f.display().to_string() })),
        )?;
        let data = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        read_ppm(&data)
    }

    /// The console after the mark.
    fn console(&self) -> Result<Vec<u8>, String> {
        let data =
            std::fs::read(&self.console).map_err(|e| format!("{}: {e}", self.console.display()))?;
        Ok(data.get(self.mark..).unwrap_or_default().to_vec())
    }

    /// Do one line of the control socket. Give true for `quit`.
    pub fn line(&mut self, line: &str) -> Result<bool, String> {
        let line = line.trim();
        let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
        let words: Vec<&str> = arg.split_whitespace().collect();
        let num = |i: usize, default: Option<u32>| -> Result<u32, String> {
            match words.get(i) {
                Some(w) => w.parse().map_err(|_| format!("bad number {w}")),
                None => default.ok_or_else(|| format!("{verb} needs more numbers")),
            }
        };
        match verb {
            "" => {}
            _ if verb.starts_with('#') => {}
            "send" => self.send(&text(arg)?)?,
            "wait" => {
                let t = text(arg)?;
                let end = Instant::now() + self.timeout;
                loop {
                    let c = self.console()?;
                    if let Some(i) = find(&c, &t) {
                        self.mark += i + t.len();
                        break;
                    }
                    if Instant::now() > end {
                        return Err(format!(
                            "no {:?} on the console in {} s",
                            String::from_utf8_lossy(&t),
                            self.timeout.as_secs()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
            "absent" => {
                let t = text(arg)?;
                if find(&self.console()?, &t).is_some() {
                    return Err(format!("the console has {:?}", String::from_utf8_lossy(&t)));
                }
            }
            "timeout" => self.timeout = seconds(words.first().copied().unwrap_or(""))?,
            "pause" => std::thread::sleep(seconds(words.first().copied().unwrap_or(""))?),
            "mouse" => {
                let b = u8::try_from(num(2, Some(0))?).map_err(|e| e.to_string())?;
                self.mouse(num(0, None)?, num(1, None)?, b)?;
            }
            "click" => {
                let (x, y) = (num(0, None)?, num(1, None)?);
                let b = u8::try_from(num(2, Some(1))?).map_err(|e| e.to_string())?;
                self.mouse(x, y, 0)?;
                self.mouse(x, y, b)?;
                self.mouse(x, y, 0)?;
            }
            "quiet" => {
                let still = Duration::from_millis(u64::from(num(0, Some(500))?));
                let limit = Duration::from_secs(u64::from(num(1, Some(30))?));
                let r = rect_of(words.get(2..).unwrap_or_default())?;
                let start = Instant::now();
                let mut img = crop(&self.screen()?, r);
                let mut last = img.sha256();
                let mut since = Instant::now();
                // On TCG, a program such as fontsel can take many seconds before it draws, and its
                // window is still and blank in that time. A blank window is thus not still.
                while since.elapsed() < still || blank_window(&img) {
                    if start.elapsed() > limit {
                        if blank_window(&img) {
                            break;
                        }
                        return Err(format!("the screen did not stop in {} s", limit.as_secs()));
                    }
                    std::thread::sleep(LOOK_GAP);
                    img = crop(&self.screen()?, r);
                    let now = img.sha256();
                    if now != last {
                        last = now;
                        since = Instant::now();
                    }
                }
            }
            "snap" => {
                let file = words.first().ok_or("snap needs a file")?;
                let r = rect_of(words.get(1..).unwrap_or_default())?;
                let img = crop(&self.screen()?, r);
                std::fs::write(file, gui::png(&img)).map_err(|e| format!("{file}: {e}"))?;
            }
            "regs" => {
                let r = self.qmp.execute(
                    "human-monitor-command",
                    Some(json!({ "command-line": "info registers -a" })),
                )?;
                let f = self.work.join("regs.txt");
                std::fs::write(&f, r.as_str().unwrap_or_default())
                    .map_err(|e| format!("{}: {e}", f.display()))?;
            }
            "quit" => return Ok(true),
            _ => return Err(format!("unknown step {verb}")),
        }
        Ok(false)
    }
}

/// The index of `needle` in `hay`.
fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Serve the control socket at `path` with `machine` until the stop flag is set. Each line gets
/// `ok`, or the error.
fn serve(
    listener: &std::os::unix::net::UnixListener,
    machine: &mut Machine,
    stop: &std::sync::atomic::AtomicBool,
) {
    use std::sync::atomic::Ordering;
    for conn in listener.incoming() {
        if stop.load(Ordering::SeqCst) {
            break;
        }
        let Ok(conn) = conn else { continue };
        let Ok(read) = conn.try_clone() else { continue };
        let mut w = conn;
        for line in BufReader::new(read).lines() {
            let Ok(line) = line else { break };
            let answer = match machine.line(&line) {
                Ok(_) => "ok".to_string(),
                Err(e) => format!("error: {e}"),
            };
            if writeln!(w, "{answer}").is_err() {
                break;
            }
        }
    }
}

/// Boot the root image of `req` under QEMU with a screen, and run `at_shell` with the path of
/// the control socket at the first shell prompt. The screen at the end goes to `screen.png` in
/// the work directory.
pub fn run_with(req: &boot::Request, mut at_shell: impl FnMut(&Path)) -> Result<Outcome, String> {
    use std::process::{Command, Stdio};
    let work = &req.work;
    std::fs::create_dir_all(work).map_err(|e| format!("{}: {e}", work.display()))?;
    let _ = std::fs::remove_file(boot::overlay(req));
    let oc = boot::overlay_command(req);
    let status = Command::new(&oc[0])
        .args(&oc[1..])
        .status()
        .map_err(|e| format!("cannot run qemu-img: {e}"))?;
    if !status.success() {
        return Err(format!("{} failed", oc.join(" ")));
    }
    let control = control_path(work);
    for p in [&control, &qmp_path(work)] {
        let _ = std::fs::remove_file(p);
    }
    let qc = qemu_command(req);
    std::fs::write(work.join("qemu.cmd"), format!("{}\n", qc.join(" ")))
        .map_err(|e| format!("{}: {e}", work.display()))?;
    let log_path = work.join("console.log");
    let mut log =
        std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let stderr = std::fs::File::create(work.join("qemu.stderr"))
        .map_err(|e| format!("{}: {e}", work.display()))?;
    let mut child = Command::new(&qc[0])
        .args(&qc[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", qc[0]))?;
    let mut failed = None;
    let driven = boot::drive(
        &mut child,
        &mut log,
        &req.bootargs,
        req.commands.clone(),
        req.timeout,
        || {},
        || {
            if let Err(e) = at_prompt(work, &control, &mut at_shell) {
                failed = Some(e);
            }
        },
    );
    let _ = std::fs::remove_file(&control);
    let _ = std::fs::remove_file(qmp_path(work));
    let _ = std::fs::remove_file(work.join("screen.ppm"));
    if !req.keep {
        let _ = std::fs::remove_file(boot::overlay(req));
    }
    let outcome = driven?.outcome();
    boot::write_steps(&work.join("steps.json"), &outcome)?;
    if let Some(e) = failed {
        return Err(e);
    }
    Ok(outcome)
}

/// The work at the first shell prompt: serve the control socket, click the console window, run
/// `at_shell`, and write the screen.
fn at_prompt(work: &Path, control: &Path, at_shell: &mut impl FnMut(&Path)) -> Result<(), String> {
    use std::sync::atomic::{AtomicBool, Ordering};
    let qmp = Qmp::connect(&qmp_path(work), Duration::from_secs(10))?;
    let mut machine = Machine::new(qmp, work);
    let listener = std::os::unix::net::UnixListener::bind(control)
        .map_err(|e| format!("{}: {e}", control.display()))?;
    let stop = AtomicBool::new(false);
    std::thread::scope(|s| {
        let server = s.spawn(|| {
            serve(&listener, &mut machine, &stop);
            machine
        });
        if let Err(e) = crate::h9boot::click_console(control) {
            eprintln!("r9: the click in the console window of rio failed: {e}");
        }
        at_shell(control);
        // The serial keys go to the rio window that has the focus. The steps can close that
        // window, and then the halt command goes nowhere. A click gives the focus to the
        // console window again.
        if let Err(e) = crate::h9boot::click_console(control) {
            eprintln!("r9: the click in the console window of rio failed: {e}");
        }
        stop.store(true, Ordering::SeqCst);
        // A connection wakes the server, which then sees the flag.
        let _ = std::os::unix::net::UnixStream::connect(control);
        let mut machine = server.join().map_err(|_| "the control server failed")?;
        let img = machine.screen()?;
        let f = work.join("screen.png");
        std::fs::write(&f, gui::png(&img)).map_err(|e| format!("{}: {e}", f.display()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_of_the_runes() {
        assert_eq!(qcode('a'), Some(("a", Modifier::None)));
        assert_eq!(qcode('Q'), Some(("q", Modifier::Shift)));
        assert_eq!(qcode('7'), Some(("7", Modifier::None)));
        assert_eq!(qcode('&'), Some(("7", Modifier::Shift)));
        assert_eq!(qcode(')'), Some(("0", Modifier::Shift)));
        assert_eq!(qcode('\''), Some(("apostrophe", Modifier::None)));
        assert_eq!(qcode('"'), Some(("apostrophe", Modifier::Shift)));
        assert_eq!(qcode('|'), Some(("backslash", Modifier::Shift)));
        assert_eq!(qcode('\n'), Some(("ret", Modifier::None)));
        assert_eq!(qcode('\u{4}'), Some(("d", Modifier::Ctrl)));
        assert_eq!(qcode('\u{7f}'), Some(("delete", Modifier::None)));
        assert_eq!(qcode('\u{1b}'), Some(("esc", Modifier::None)));
        assert_eq!(qcode('\u{f018}'), Some(("end", Modifier::None)));
        assert_eq!(qcode('\u{f00e}'), Some(("up", Modifier::None)));
        assert_eq!(qcode('\u{80}'), Some(("down", Modifier::None)));
        assert_eq!(qcode('\u{f001}'), Some(("f1", Modifier::None)));
        assert_eq!(qcode('é'), None);
        // Each printable ASCII character has a key.
        for c in ' '..='~' {
            assert!(qcode(c).is_some(), "{c:?}");
        }
    }

    #[test]
    fn the_absolute_axis_gives_the_pixel_back() {
        for size in [768, 1024] {
            for p in 0..size {
                let v = abs_value(p, size);
                assert!(v < ABS_SPAN);
                assert_eq!((v * size + ABS_SPAN / 2) / ABS_SPAN, p);
            }
        }
        assert_eq!(abs_value(300, 1024), 9600);
        assert_eq!(abs_value(300, 768), 12800);
    }

    #[test]
    fn the_text_of_a_line() {
        assert_eq!(text("  R9-GUI-EXIT ").unwrap(), b"R9-GUI-EXIT");
        assert_eq!(text(r#""a\nb\x7f\\\"""#).unwrap(), b"a\nb\x7f\\\"");
        assert!(text("\"open").is_err());
        assert_eq!(
            text(&gui::send("q\n\u{f018}".as_bytes())[5..]).unwrap(),
            "q\n\u{f018}".as_bytes()
        );
    }

    #[test]
    fn a_blank_window() {
        let (w, h) = (100u32, 60u32);
        let mut rgb = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let border = x < 4 || y < 4 || x >= w - 4 || y >= h - 4;
                let scroll = x == 16 && (5..h - 5).contains(&y);
                rgb.extend_from_slice(match (border, scroll) {
                    (true, _) => &[85, 170, 170],
                    (false, true) => &[153, 153, 153],
                    _ => &[255, 255, 255],
                });
            }
        }
        let mut img = Image {
            width: w,
            height: h,
            rgb,
        };
        assert!(blank_window(&img));
        for p in img
            .rgb
            .as_chunks_mut::<3>()
            .0
            .iter_mut()
            .skip(30 * 100)
            .take(600)
        {
            p.copy_from_slice(&[0, 0, 0]);
        }
        assert!(!blank_window(&img));
    }

    #[test]
    fn a_ppm_and_a_crop() {
        let mut data = b"P6\n3 2\n255\n".to_vec();
        data.extend(0u8..18);
        let img = read_ppm(&data).unwrap();
        assert_eq!((img.width, img.height), (3, 2));
        assert_eq!(img.pixel(2, 1), [15, 16, 17]);
        let c = crop(&img, Some([1, 0, 3, 2]));
        assert_eq!((c.width, c.height), (2, 2));
        assert_eq!(c.rgb, vec![3, 4, 5, 6, 7, 8, 12, 13, 14, 15, 16, 17]);
        assert!(read_ppm(b"P5\n1 1\n255\n\0").is_err());
        assert!(read_ppm(b"P6\n2 2\n255\n\0").is_err());
    }

    #[test]
    fn the_boot_that_sets_the_screen() {
        let c = prepare_commands(true);
        assert!(
            c.iter()
                .any(|l| l.contains("echo 'monitor=vesa'; echo 'vgasize=1024x768x32'"))
        );
        assert!(c.last().unwrap().contains("tar xf $r9in"));
        assert!(!prepare_commands(false).iter().any(|l| l.contains("r9in")));
    }

    #[test]
    fn the_steps_that_need_no_machine() {
        assert_eq!(rect_of(&[]).unwrap(), None);
        assert_eq!(rect_of(&["1", "2", "3", "4"]).unwrap(), Some([1, 2, 3, 4]));
        assert!(rect_of(&["3", "2", "1", "4"]).is_err());
        assert_eq!(seconds("1.5").unwrap(), Duration::from_millis(1500));
        assert!(seconds("x").is_err());
        assert_eq!(find(b"abc R9-GUI-EXIT 0", b"R9-GUI-EXIT"), Some(4));
        assert_eq!(find(b"abc", b"x"), None);
    }
}
