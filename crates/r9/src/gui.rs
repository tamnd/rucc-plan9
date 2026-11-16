//! `r9 gui run` and `r9 gui compare`: the pixel oracle of plan 12.6.
//!
//! A GUI script, `gui/<prog>.r9`, starts one program in a `rio` window of a fixed size, gives it
//! input, waits until the window is still, and records images of the window. `r9 gui run` boots
//! the 9h kernel with a screen, as `r9 9h boot --snap` does, and gives the steps of the script to
//! the monitor on its control socket at the first shell prompt of the console window. The input
//! comes from the host and not from a process in the guest, because the keys and the mouse of the
//! 9h machine go to the current window, as the keys and the mouse of a person do.
//!
//! The steps, one on each line, with `#` for a comment:
//!
//! - `class exact|masked|smoke` is the class of the script (plan 02.7), `exact` by default;
//! - `window WxH` is the size of the window, 640x480 by default, with its border;
//! - `run CMD` starts CMD, an rc command, in a new window at the bottom right of the screen;
//! - `quiet [MS [SECS]]` waits until the window did not change for MS milliseconds (500 by
//!   default), for no more than SECS seconds (30 by default);
//! - `snap NAME` records the image of the window as NAME;
//! - `mouse B at X,Y` clicks button B (1, 2 or 3) at X,Y in the window;
//! - `mouse chord A+B at X,Y` pushes button A, then button B, at X,Y, and then lets go of both;
//! - `move X,Y` moves the mouse to X,Y in the window;
//! - `drag B X0,Y0 X1,Y1` pushes button B at X0,Y0, moves to X1,Y1 and lets go;
//! - `keys TEXT` types TEXT, a quoted string with the escapes `\n`, `\t`, `\\`, `\"` and `\xHH`;
//! - `pause SECS` waits;
//! - `mask X0 Y0 X1 Y1` adds a rectangle of the window that `r9 gui compare` does not compare in
//!   the next snaps, and `mask none` removes all of them;
//! - `exit KEY` types KEY (`Del`, `Esc` or a quoted string), and then the program must stop with
//!   an empty status in 60 seconds. `exit none` types nothing, for a program that stops from a
//!   menu: the steps before it choose the item of the menu.
//!
//! The coordinates are in the window, with 0,0 at the top left of its border. The output
//! directory has a PNG file for each snap, the screen at the end in `screen.png`, and `gui.json`,
//! which has the SHA-256 hash of the pixels of each snap and its masks.

use crate::h9boot;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt::Write as _;
use std::path::Path;

/// The size of the screen of the 9h machine.
pub const SCREEN: (u32, u32) = (1024, 768);

/// A point in the console window of `rio` that a window of a script does not cover. `riostart`
/// puts the console window 600 by 400 pixels at 32, 48 or 64 pixels from the corner, and `stats`
/// at 0,0 to 161,117. A window of a script is at the bottom right of the screen, so it covers this
/// point only when it is wider than 924 and taller than 368 pixels.
pub const CONSOLE_POINT: (u32, u32) = (100, 400);

/// The text that the run file writes to the kernel console, with the status of the program after
/// it. The run file writes it as `R9-GUI^-EXIT`, so that the line that the console window types
/// does not have it.
pub const EXIT_MARK: &str = "R9-GUI-EXIT";

/// The time in seconds that the program can take to stop after `exit`.
const EXIT_SECS: u32 = 60;

/// The rc file in the guest that runs the program.
const RUN_FILE: &str = "/tmp/r9gui.rc";

/// The class of a script (plan 02.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    /// The images must be equal.
    Exact,
    /// The images must be equal outside the masks.
    Masked,
    /// The program must start, take the input and stop. The images are not compared.
    Smoke,
}

/// One step of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Start an rc command in a new window.
    Run(String),
    /// Wait until the window did not change for the milliseconds, for no more than the seconds.
    Quiet(u64, u64),
    /// Record the image of the window with this name.
    Snap(String),
    /// Push these buttons, one after the other, at the point, then let go of all of them.
    Mouse(Vec<u8>, (u32, u32)),
    /// Move the mouse to the point.
    Move((u32, u32)),
    /// Push the button at the first point, move to the second point and let go.
    Drag(u8, (u32, u32), (u32, u32)),
    /// Type the bytes.
    Keys(Vec<u8>),
    /// Wait for the seconds.
    Pause(u64),
    /// Add a mask, or remove all of them with None.
    Mask(Option<[u32; 4]>),
    /// Type the bytes, and then the program must stop.
    Exit(Vec<u8>),
}

/// A GUI script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// The class.
    pub class: Class,
    /// The width and height of the window.
    pub size: (u32, u32),
    /// The steps with their line numbers.
    pub steps: Vec<(usize, Step)>,
}

impl Script {
    /// The rectangle of the window on the screen: x0, y0, x1, y1.
    #[must_use]
    pub fn rect(&self) -> [u32; 4] {
        let (w, h) = self.size;
        [SCREEN.0 - w, SCREEN.1 - h, SCREEN.0, SCREEN.1]
    }

    /// Whether the script has an `exit` step.
    #[must_use]
    pub fn exits(&self) -> bool {
        self.steps.iter().any(|(_, s)| matches!(s, Step::Exit(_)))
    }
}

/// The bytes of a quoted string with the escapes `\n`, `\t`, `\\`, `\"` and `\xHH`.
fn quoted(s: &str) -> Option<Vec<u8>> {
    let inner = s.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = Vec::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '"' {
            return None;
        }
        if c != '\\' {
            let mut b = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut b).as_bytes());
            continue;
        }
        match chars.next()? {
            'n' => out.push(b'\n'),
            't' => out.push(b'\t'),
            '\\' => out.push(b'\\'),
            '"' => out.push(b'"'),
            'x' => {
                let h: String = chars.by_ref().take(2).collect();
                if h.len() != 2 {
                    return None;
                }
                out.push(u8::from_str_radix(&h, 16).ok()?);
            }
            _ => return None,
        }
    }
    Some(out)
}

/// The bytes of a key name or a quoted string.
fn key(s: &str) -> Option<Vec<u8>> {
    match s {
        "Del" => Some(vec![0x7f]),
        "Esc" => Some(vec![0x1b]),
        "none" => Some(Vec::new()),
        _ => quoted(s),
    }
}

/// The point of `X,Y`.
fn point(s: &str) -> Option<(u32, u32)> {
    let (x, y) = s.split_once(',')?;
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// The button of a number 1, 2 or 3.
fn button(s: &str) -> Option<u8> {
    match s {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(4),
        _ => None,
    }
}

/// The numbers of the words, all or none.
fn numbers(words: &[&str]) -> Option<Vec<u64>> {
    words.iter().map(|w| w.parse().ok()).collect()
}

/// One step of the words of a line, or None when the words are wrong.
fn step(verb: &str, rest: &str) -> Option<Step> {
    let words: Vec<&str> = rest.split_whitespace().collect();
    Some(match (verb, &words[..]) {
        ("run", [_, ..]) => Step::Run(rest.to_string()),
        ("quiet", _) => match numbers(&words)?[..] {
            [] => Step::Quiet(500, 30),
            [ms] => Step::Quiet(ms, 30),
            [ms, secs] => Step::Quiet(ms, secs),
            _ => return None,
        },
        ("snap", [name])
            if name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') =>
        {
            Step::Snap((*name).to_string())
        }
        ("mouse", ["chord", bs, "at", p]) => {
            let bs: Option<Vec<u8>> = bs.split('+').map(button).collect();
            let bs = bs?;
            if bs.len() < 2 {
                return None;
            }
            Step::Mouse(bs, point(p)?)
        }
        ("mouse", [b, "at", p]) => Step::Mouse(vec![button(b)?], point(p)?),
        ("move", [p]) => Step::Move(point(p)?),
        ("drag", [b, p, q]) => Step::Drag(button(b)?, point(p)?, point(q)?),
        ("keys", [_, ..]) => Step::Keys(quoted(rest)?),
        ("pause", [n]) => Step::Pause(n.parse().ok()?),
        ("mask", ["none"]) => Step::Mask(None),
        ("mask", _) => match numbers(&words)?[..] {
            [x0, y0, x1, y1] if x0 < x1 && y0 < y1 => Step::Mask(Some(
                [x0, y0, x1, y1].map(|v| u32::try_from(v).unwrap_or(u32::MAX)),
            )),
            _ => return None,
        },
        ("exit", [_, ..]) => Step::Exit(key(rest)?),
        _ => return None,
    })
}

/// Read a GUI script.
pub fn parse(src: &str) -> Result<Script, String> {
    let mut script = Script {
        class: Class::Exact,
        size: (640, 480),
        steps: Vec::new(),
    };
    let mut runs = 0;
    for (i, line) in src.lines().enumerate() {
        let n = i + 1;
        let line = line.trim();
        // A comment starts at a # that is not in a quoted string.
        let mut quote = false;
        let end = line
            .char_indices()
            .find(|&(_, c)| {
                if c == '"' {
                    quote = !quote;
                }
                c == '#' && !quote
            })
            .map_or(line.len(), |(at, _)| at);
        let line = line[..end].trim();
        if line.is_empty() {
            continue;
        }
        let (verb, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        match verb {
            "class" => {
                script.class = match rest {
                    "exact" => Class::Exact,
                    "masked" => Class::Masked,
                    "smoke" => Class::Smoke,
                    _ => return Err(format!("line {n}: give class exact, masked or smoke")),
                }
            }
            "window" => {
                let size = rest
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)));
                let Some((w, h)) = size else {
                    return Err(format!("line {n}: give window WxH"));
                };
                if !(16..=SCREEN.0).contains(&w) || !(16..=SCREEN.1).contains(&h) {
                    return Err(format!(
                        "line {n}: the window must be from 16x16 to {}x{}",
                        SCREEN.0, SCREEN.1
                    ));
                }
                if w > SCREEN.0 - CONSOLE_POINT.0 && h > SCREEN.1 - CONSOLE_POINT.1 {
                    return Err(format!(
                        "line {n}: a window of {w}x{h} covers the console window"
                    ));
                }
                if runs > 0 {
                    return Err(format!("line {n}: give window before run"));
                }
                script.size = (w, h);
            }
            _ => {
                let s =
                    step(verb, rest).ok_or_else(|| format!("line {n}: {line:?} is not a step"))?;
                if matches!(s, Step::Run(_)) {
                    runs += 1;
                    if runs > 1 {
                        return Err(format!("line {n}: a script runs one program"));
                    }
                } else if runs == 0 && !matches!(s, Step::Mask(_) | Step::Pause(_)) {
                    return Err(format!("line {n}: give run before {verb}"));
                }
                script.steps.push((n, s));
            }
        }
    }
    if runs == 0 {
        return Err("the script has no run step".into());
    }
    let (w, h) = script.size;
    for (n, s) in &script.steps {
        let points: Vec<(u32, u32)> = match s {
            Step::Mouse(_, p) | Step::Move(p) => vec![*p],
            Step::Drag(_, p, q) => vec![*p, *q],
            Step::Mask(Some([_, _, x1, y1])) => vec![(x1 - 1, y1 - 1)],
            _ => continue,
        };
        if points.iter().any(|&(x, y)| x >= w || y >= h) {
            return Err(format!(
                "line {n}: the point is not in the window of {w}x{h}"
            ));
        }
    }
    Ok(script)
}

/// Text for the `send` step of the monitor, in double quotes with escapes.
fn send(bytes: &[u8]) -> String {
    let mut s = String::from("send \"");
    for &b in bytes {
        match b {
            b'"' => s.push_str("\\\""),
            b'\\' => s.push_str("\\\\"),
            b'\n' => s.push_str("\\n"),
            0x20..=0x7e => s.push(char::from(b)),
            _ => {
                let _ = write!(s, "\\x{b:02x}");
            }
        }
    }
    s.push('"');
    s
}

/// The rc string of `s`, in single quotes.
fn rc_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The lines for the control socket of the monitor that do one step. `snaps` is the directory
/// for the PNG files of the snaps.
#[must_use]
pub fn control_lines(step: &Step, rect: [u32; 4], snaps: &Path) -> Vec<String> {
    let [x0, y0, x1, y1] = rect;
    let at = |(x, y): (u32, u32)| (x0 + x, y0 + y);
    let mouse = |p: (u32, u32), b: u8| {
        let (x, y) = at(p);
        format!("mouse {x} {y} {b}")
    };
    match step {
        Step::Run(cmd) => vec![send(
            format!(
                "echo {} > {RUN_FILE}; echo {} >> {RUN_FILE}; window -r {x0} {y0} {x1} {y1} rc {RUN_FILE}\n",
                rc_quote(cmd),
                rc_quote("echo R9-GUI^-EXIT $status > '#c/cons'"),
            )
            .as_bytes(),
        )],
        Step::Quiet(ms, secs) => vec![format!("quiet {ms} {secs} {x0} {y0} {x1} {y1}")],
        Step::Snap(name) => vec![format!(
            "snap {} {x0} {y0} {x1} {y1}",
            snaps.join(format!("{name}.png")).display()
        )],
        Step::Mouse(buttons, p) => {
            let mut v = vec![mouse(*p, 0)];
            let mut down = 0;
            for b in buttons {
                down |= b;
                v.push(mouse(*p, down));
            }
            v.push(mouse(*p, 0));
            v
        }
        Step::Move(p) => vec![mouse(*p, 0)],
        Step::Drag(b, p, q) => vec![
            mouse(*p, 0),
            mouse(*p, *b),
            mouse(*q, *b),
            mouse(*q, 0),
        ],
        Step::Keys(k) | Step::Exit(k) if k.is_empty() => Vec::new(),
        Step::Keys(k) | Step::Exit(k) => vec![send(k)],
        Step::Pause(s) => vec![format!("pause {s}")],
        Step::Mask(_) => Vec::new(),
    }
}

/// The status of the program in the console output `console`: the text after the last
/// [`EXIT_MARK`] on its line, or None when the console has no mark.
#[must_use]
pub fn exit_status(console: &str) -> Option<String> {
    let at = console.rfind(EXIT_MARK)? + EXIT_MARK.len();
    let rest = &console[at..];
    Some(rest[..rest.find('\n').unwrap_or(rest.len())].trim().to_string())
}

/// An image of 8-bit RGB pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    /// The width in pixels.
    pub width: u32,
    /// The height in pixels.
    pub height: u32,
    /// Three bytes for each pixel, row by row.
    pub rgb: Vec<u8>,
}

impl Image {
    /// The SHA-256 hash of the pixels, in hex.
    #[must_use]
    pub fn sha256(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.width.to_be_bytes());
        h.update(self.height.to_be_bytes());
        h.update(&self.rgb);
        h.finalize().iter().fold(String::new(), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
    }

    /// The pixel at x, y.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> [u8; 3] {
        let at = (y as usize * self.width as usize + x as usize) * 3;
        [self.rgb[at], self.rgb[at + 1], self.rgb[at + 2]]
    }
}

fn crc32(crc: u32, data: &[u8]) -> u32 {
    let mut c = !crc;
    for &b in data {
        c ^= u32::from(b);
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xedb8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += u32::from(x);
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    b << 16 | a
}

/// The PNG file of an image, with stored deflate blocks.
#[must_use]
pub fn png(img: &Image) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        out.extend_from_slice(&crc32(crc32(0, &kind), data).to_be_bytes());
    }
    let row = img.width as usize * 3;
    let mut raw = Vec::with_capacity((row + 1) * img.height as usize);
    for r in img.rgb.chunks(row.max(1)).take(img.height as usize) {
        raw.push(0);
        raw.extend_from_slice(r);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(0xffff).peekable();
    if blocks.peek().is_none() {
        z.extend_from_slice(&[1, 0, 0, 0xff, 0xff]);
    }
    while let Some(b) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        let n = u16::try_from(b.len()).unwrap_or(0);
        z.extend_from_slice(&n.to_le_bytes());
        z.extend_from_slice(&(!n).to_le_bytes());
        z.extend_from_slice(b);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&img.width.to_be_bytes());
    ihdr.extend_from_slice(&img.height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &ihdr);
    chunk(&mut out, *b"IDAT", &z);
    chunk(&mut out, *b"IEND", &[]);
    out
}

/// Read a PNG file of 8-bit RGB pixels with stored deflate blocks and no row filters, as the 9h
/// monitor and [`png`] write it.
pub fn read_png(data: &[u8]) -> Result<Image, String> {
    let bad = |why: &str| format!("not a PNG file of the 9h monitor: {why}");
    let mut rest = data
        .strip_prefix(b"\x89PNG\r\n\x1a\n")
        .ok_or_else(|| bad("no signature"))?;
    let (mut width, mut height) = (0u32, 0u32);
    let mut z = Vec::new();
    while rest.len() >= 12 {
        let len = u32::from_be_bytes(rest[..4].try_into().unwrap_or_default()) as usize;
        let kind = &rest[4..8];
        let body = rest.get(8..8 + len).ok_or_else(|| bad("a short chunk"))?;
        match kind {
            b"IHDR" => {
                if body.len() != 13 || body[8..] != [8, 2, 0, 0, 0] {
                    return Err(bad("not 8-bit RGB"));
                }
                width = u32::from_be_bytes(body[..4].try_into().unwrap_or_default());
                height = u32::from_be_bytes(body[4..8].try_into().unwrap_or_default());
            }
            b"IDAT" => z.extend_from_slice(body),
            b"IEND" => break,
            _ => {}
        }
        rest = rest.get(12 + len..).ok_or_else(|| bad("a short chunk"))?;
    }
    let mut at = 2;
    let mut raw = Vec::new();
    loop {
        let head = *z.get(at).ok_or_else(|| bad("a short stream"))?;
        if head & 6 != 0 {
            return Err(bad("a compressed block"));
        }
        let n = z
            .get(at + 1..at + 3)
            .map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
            .ok_or_else(|| bad("a short stream"))?;
        raw.extend_from_slice(
            z.get(at + 5..at + 5 + n)
                .ok_or_else(|| bad("a short block"))?,
        );
        at += 5 + n;
        if head & 1 != 0 {
            break;
        }
    }
    let row = width as usize * 3;
    if raw.len() != (row + 1) * height as usize {
        return Err(bad("the size of the pixels is wrong"));
    }
    let mut rgb = Vec::with_capacity(row * height as usize);
    for r in raw.chunks(row + 1) {
        if r[0] != 0 {
            return Err(bad("a row filter"));
        }
        rgb.extend_from_slice(&r[1..]);
    }
    Ok(Image { width, height, rgb })
}

/// One snap in `gui.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snap {
    /// The name of the snap.
    pub name: String,
    /// The PNG file, in the output directory.
    pub file: String,
    /// The SHA-256 hash of the pixels.
    pub sha256: String,
    /// The masks: x0, y0, x1, y1 in the window.
    pub masks: Vec<[u32; 4]>,
}

/// `gui.json`: what came of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    /// The script.
    pub script: String,
    /// The class of the script.
    pub class: Class,
    /// The rectangle of the window on the screen.
    pub window: [u32; 4],
    /// The snaps in order.
    pub snaps: Vec<Snap>,
    /// Whether each step and the boot passed.
    pub passed: bool,
    /// Why the run failed.
    pub failure: Option<String>,
}

/// Give the lines to the control socket at `path`, one at a time, and fail at the first answer
/// that is not `ok`.
#[cfg(unix)]
fn give(path: &Path, lines: &[String]) -> Result<(), String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;
    let s = UnixStream::connect(path).map_err(|e| format!("{}: {e}", path.display()))?;
    s.set_read_timeout(Some(std::time::Duration::from_secs(120)))
        .map_err(|e| e.to_string())?;
    let mut r = BufReader::new(&s);
    for l in lines {
        (&s).write_all(format!("{l}\n").as_bytes())
            .map_err(|e| e.to_string())?;
        let mut answer = String::new();
        r.read_line(&mut answer).map_err(|e| e.to_string())?;
        if answer.trim() != "ok" {
            return Err(format!("{l}: {}", answer.trim()));
        }
    }
    Ok(())
}

#[cfg(not(unix))]
fn give(_: &Path, _: &[String]) -> Result<(), String> {
    Err("the control socket needs a Unix host".into())
}

/// Run the steps of `script` on the control socket at `path`. The snaps go to `out`.
fn run_steps(script: &Script, path: &Path, out: &Path) -> Result<Vec<Snap>, String> {
    let rect = script.rect();
    let mut masks = Vec::new();
    let mut snaps = Vec::new();
    for (n, s) in &script.steps {
        if let Step::Mask(m) = s {
            match m {
                Some(r) => masks.push(*r),
                None => masks.clear(),
            }
            continue;
        }
        give(path, &control_lines(s, rect, out)).map_err(|e| format!("line {n}: {e}"))?;
        if let Step::Snap(name) = s {
            let file = format!("{name}.png");
            let f = out.join(&file);
            let data = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            let img = read_png(&data).map_err(|e| format!("{}: {e}", f.display()))?;
            snaps.push(Snap {
                name: name.clone(),
                file,
                sha256: img.sha256(),
                masks: masks.clone(),
            });
        }
    }
    if script.exits() {
        // Rio drops the keys that come while the window of the program closes, so the console
        // gets the click and the keys only after the run file wrote the mark.
        give(
            path,
            &[
                format!("timeout {EXIT_SECS}"),
                format!("wait {EXIT_MARK}"),
                "pause 1".into(),
            ],
        )
        .map_err(|e| format!("the exit: {e}"))?;
    }
    let (x, y) = CONSOLE_POINT;
    give(path, &[format!("click {x} {y}")]).map_err(|e| format!("the console click: {e}"))?;
    Ok(snaps)
}

/// Run the script at `file` on the 9h machine of `req`. `req.snap` must be set, so that the
/// machine has a screen and a control socket. The output goes to `req.work`.
pub fn run(req: &h9boot::Request, file: &Path) -> Result<Record, String> {
    let src = std::fs::read_to_string(file).map_err(|e| format!("{}: {e}", file.display()))?;
    let script = parse(&src).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut req = req.clone();
    req.commands = Vec::new();
    std::fs::create_dir_all(&req.work).map_err(|e| format!("{}: {e}", req.work.display()))?;
    let mut steps = Err("the guest gave no shell prompt".to_string());
    let outcome = h9boot::run_with(&req, |path| {
        steps = run_steps(&script, path, &req.work);
    })?;
    let status = if script.exits() {
        let f = req.work.join("console.log");
        let console = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
        exit_status(&console)
    } else {
        Some(String::new())
    };
    let failure = match (&steps, status) {
        (Err(e), _) => Some(e.clone()),
        (Ok(_), _) if !outcome.passed => Some("the boot did not pass".to_string()),
        (Ok(_), None) => Some(format!("the console has no {EXIT_MARK} line")),
        (Ok(_), Some(s)) if !s.is_empty() => Some(format!("the program did not stop well: {s}")),
        (Ok(_), Some(_)) => None,
    };
    let record = Record {
        script: file.display().to_string(),
        class: script.class,
        window: script.rect(),
        snaps: steps.unwrap_or_default(),
        passed: failure.is_none(),
        failure,
    };
    let json = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    let f = req.work.join("gui.json");
    std::fs::write(&f, json + "\n").map_err(|e| format!("{}: {e}", f.display()))?;
    Ok(record)
}

/// Read `gui.json` in the directory `dir`.
pub fn load(dir: &Path) -> Result<Record, String> {
    let f = dir.join("gui.json");
    let text = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("{}: {e}", f.display()))
}

/// The first difference of two runs of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Difference {
    /// The name of the snap.
    pub snap: String,
    /// What is different.
    pub what: String,
    /// The number of pixels that are different, outside the masks.
    pub pixels: u64,
    /// The rectangle that has all the different pixels.
    pub bounds: Option<[u32; 4]>,
}

/// Whether x, y is in one of the masks.
fn masked(masks: &[[u32; 4]], x: u32, y: u32) -> bool {
    masks
        .iter()
        .any(|&[x0, y0, x1, y1]| (x0..x1).contains(&x) && (y0..y1).contains(&y))
}

/// Compare two images outside the masks. Give the number of pixels that are different, the
/// rectangle that has them, and the image of the difference: the XOR of the pixels, with the
/// masks in gray.
#[must_use]
pub fn compare_images(
    want: &Image,
    got: &Image,
    masks: &[[u32; 4]],
) -> (u64, Option<[u32; 4]>, Image) {
    let mut diff = Image {
        width: want.width,
        height: want.height,
        rgb: vec![0; want.rgb.len()],
    };
    let mut count = 0;
    let mut bounds: Option<[u32; 4]> = None;
    for y in 0..want.height {
        for x in 0..want.width {
            let at = (y as usize * want.width as usize + x as usize) * 3;
            if masked(masks, x, y) {
                diff.rgb[at..at + 3].copy_from_slice(&[0x80, 0x80, 0x80]);
                continue;
            }
            let (pw, pg) = (want.pixel(x, y), got.pixel(x, y));
            if pw == pg {
                continue;
            }
            count += 1;
            for i in 0..3 {
                diff.rgb[at + i] = pw[i] ^ pg[i];
            }
            let r = bounds.get_or_insert([x, y, x + 1, y + 1]);
            *r = [r[0].min(x), r[1].min(y), r[2].max(x + 1), r[3].max(y + 1)];
        }
    }
    (count, bounds, diff)
}

/// Compare the run in `got` with the reference run in `want`, snap by snap. For the first snap
/// that is different, the image of the difference goes to `diff`, as `want.png`, `got.png` and
/// `xor.png`. Give None when the runs are the same.
pub fn compare(want: &Path, got: &Path, diff: &Path) -> Result<Option<Difference>, String> {
    let (wr, gr) = (load(want)?, load(got)?);
    let differ = |snap: &str, what: String| {
        Ok(Some(Difference {
            snap: snap.into(),
            what,
            pixels: 0,
            bounds: None,
        }))
    };
    if !wr.passed {
        return Err(format!(
            "the reference run failed: {}",
            wr.failure.unwrap_or_default()
        ));
    }
    if !gr.passed {
        return differ(
            "",
            format!("the run failed: {}", gr.failure.unwrap_or_default()),
        );
    }
    if wr.class == Class::Smoke {
        return Ok(None);
    }
    let names = |r: &Record| r.snaps.iter().map(|s| s.name.clone()).collect::<Vec<_>>();
    if names(&wr) != names(&gr) {
        return differ(
            "",
            format!("the snaps are {:?}, not {:?}", names(&gr), names(&wr)),
        );
    }
    for (sw, sg) in wr.snaps.iter().zip(&gr.snaps) {
        let masks: Vec<[u32; 4]> = match wr.class {
            Class::Exact => Vec::new(),
            _ => sw.masks.iter().chain(&sg.masks).copied().collect(),
        };
        if masks.is_empty() && sw.sha256 == sg.sha256 {
            continue;
        }
        let read = |dir: &Path, s: &Snap| {
            let f = dir.join(&s.file);
            let data = std::fs::read(&f).map_err(|e| format!("{}: {e}", f.display()))?;
            read_png(&data).map_err(|e| format!("{}: {e}", f.display()))
        };
        let (iw, ig) = (read(want, sw)?, read(got, sg)?);
        if (iw.width, iw.height) != (ig.width, ig.height) {
            return differ(
                &sw.name,
                format!(
                    "the image is {}x{}, not {}x{}",
                    ig.width, ig.height, iw.width, iw.height
                ),
            );
        }
        let (count, bounds, xor) = compare_images(&iw, &ig, &masks);
        if count == 0 {
            continue;
        }
        std::fs::create_dir_all(diff).map_err(|e| format!("{}: {e}", diff.display()))?;
        for (name, img) in [("want.png", &iw), ("got.png", &ig), ("xor.png", &xor)] {
            let f = diff.join(name);
            std::fs::write(&f, png(img)).map_err(|e| format!("{}: {e}", f.display()))?;
        }
        return Ok(Some(Difference {
            snap: sw.name.clone(),
            what: "the pixels are different".into(),
            pixels: count,
            bounds,
        }));
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEMO: &str = r#"# gui/demo.r9
class masked
window 400x300
run demo -x 'a b'   # the program
quiet
snap open
mouse 3 at 200,40
mouse chord 1+2 at 120,200
keys "hi # there\n"
mask 0 0 50 20
quiet 200 5
snap look
mask none
drag 1 10,10 20,20
exit Del
"#;

    #[test]
    fn a_script_is_read() {
        let s = parse(DEMO).unwrap();
        assert_eq!(s.class, Class::Masked);
        assert_eq!(s.size, (400, 300));
        assert_eq!(s.rect(), [624, 468, 1024, 768]);
        assert!(s.exits());
        let steps: Vec<Step> = s.steps.iter().map(|(_, s)| s.clone()).collect();
        assert_eq!(
            steps,
            vec![
                Step::Run("demo -x 'a b'".into()),
                Step::Quiet(500, 30),
                Step::Snap("open".into()),
                Step::Mouse(vec![4], (200, 40)),
                Step::Mouse(vec![1, 2], (120, 200)),
                Step::Keys(b"hi # there\n".to_vec()),
                Step::Mask(Some([0, 0, 50, 20])),
                Step::Quiet(200, 5),
                Step::Snap("look".into()),
                Step::Mask(None),
                Step::Drag(1, (10, 10), (20, 20)),
                Step::Exit(vec![0x7f]),
            ]
        );
        assert_eq!(s.steps[0].0, 4);
    }

    #[test]
    fn a_bad_script_is_not_read() {
        for (src, why) in [
            ("quiet", "give run before quiet"),
            ("run a\nrun b", "one program"),
            ("window 1000x700\nrun a", "covers the console"),
            ("window 2000x10\nrun a", "from 16x16"),
            ("run a\nwindow 300x300", "before run"),
            ("run a\nmouse 4 at 1,1", "not a step"),
            ("run a\nmouse 1 at 640,1", "not in the window"),
            ("run a\nkeys hi", "not a step"),
            ("run a\nkeys \"\\q\"", "not a step"),
            ("run a\nsnap a/b", "not a step"),
            ("run a\nmouse chord 1 at 1,1", "not a step"),
            ("class fast", "give class"),
            ("", "no run step"),
        ] {
            let e = parse(src).unwrap_err();
            assert!(e.contains(why), "{src:?}: {e}");
        }
    }

    #[test]
    fn the_status_comes_from_the_last_mark() {
        assert_eq!(exit_status("term% x\n"), None);
        assert_eq!(
            exit_status("R9-GUI-EXIT rc 12: x\nR9-GUI-EXIT \r\nterm% "),
            Some(String::new())
        );
        assert_eq!(
            exit_status("R9-GUI-EXIT \nR9-GUI-EXIT fplot 3: bad"),
            Some("fplot 3: bad".into())
        );
    }

    #[test]
    fn the_steps_become_monitor_lines() {
        let rect = [624, 468, 1024, 768];
        let snaps = Path::new("/w");
        let l = |s: Step| control_lines(&s, rect, snaps);
        assert_eq!(
            l(Step::Run("demo -x 'a b'".into())),
            vec![
                "send \"echo 'demo -x ''a b''' > /tmp/r9gui.rc; echo 'echo R9-GUI^-EXIT $status > ''#c/cons''' >> /tmp/r9gui.rc; window -r 624 468 1024 768 rc /tmp/r9gui.rc\\n\""
            ]
        );
        assert_eq!(
            l(Step::Quiet(500, 30)),
            vec!["quiet 500 30 624 468 1024 768"]
        );
        assert_eq!(
            l(Step::Snap("open".into())),
            vec!["snap /w/open.png 624 468 1024 768"]
        );
        assert_eq!(
            l(Step::Mouse(vec![1, 2], (10, 20))),
            vec![
                "mouse 634 488 0",
                "mouse 634 488 1",
                "mouse 634 488 3",
                "mouse 634 488 0"
            ]
        );
        assert_eq!(
            l(Step::Drag(4, (0, 0), (5, 5))),
            vec![
                "mouse 624 468 0",
                "mouse 624 468 4",
                "mouse 629 473 4",
                "mouse 629 473 0"
            ]
        );
        assert_eq!(
            l(Step::Keys(b"a\"\\\n\x7f".to_vec())),
            vec!["send \"a\\\"\\\\\\n\\x7f\""]
        );
        assert!(l(Step::Mask(None)).is_empty());
        assert!(l(Step::Exit(Vec::new())).is_empty());
        assert_eq!(
            parse("run a\nexit none").unwrap().steps[1].1,
            Step::Exit(Vec::new())
        );
    }

    #[test]
    fn a_png_is_written_and_read() {
        let img = Image {
            width: 3,
            height: 2,
            rgb: (0..18).collect(),
        };
        let data = png(&img);
        assert_eq!(read_png(&data).unwrap(), img);
        let big = Image {
            width: 200,
            height: 200,
            rgb: vec![7; 200 * 200 * 3],
        };
        assert_eq!(read_png(&png(&big)).unwrap(), big);
        assert!(read_png(b"GIF89a").is_err());
        assert_ne!(img.sha256(), big.sha256());
    }

    #[test]
    fn images_are_compared_outside_the_masks() {
        let a = Image {
            width: 4,
            height: 4,
            rgb: vec![0; 48],
        };
        let mut b = a.clone();
        b.rgb[(4 + 2) * 3] = 0xff;
        b.rgb[(3 * 4 + 3) * 3 + 1] = 0x0f;
        let (n, bounds, x) = compare_images(&a, &b, &[]);
        assert_eq!((n, bounds), (2, Some([2, 1, 4, 4])));
        assert_eq!(x.pixel(2, 1), [0xff, 0, 0]);
        let (n, bounds, x) = compare_images(&a, &b, &[[3, 3, 4, 4]]);
        assert_eq!((n, bounds), (1, Some([2, 1, 3, 2])));
        assert_eq!(x.pixel(3, 3), [0x80, 0x80, 0x80]);
    }

    #[test]
    fn two_runs_are_compared() {
        let dir = std::env::temp_dir().join(format!("r9-gui-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let img = Image {
            width: 2,
            height: 2,
            rgb: vec![9; 12],
        };
        let mut other = img.clone();
        other.rgb[0] = 1;
        let write = |name: &str, i: &Image, masks: Vec<[u32; 4]>| {
            let d = dir.join(name);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("open.png"), png(i)).unwrap();
            let r = Record {
                script: "gui/demo.r9".into(),
                class: Class::Masked,
                window: [0, 0, 2, 2],
                snaps: vec![Snap {
                    name: "open".into(),
                    file: "open.png".into(),
                    sha256: i.sha256(),
                    masks,
                }],
                passed: true,
                failure: None,
            };
            std::fs::write(d.join("gui.json"), serde_json::to_string(&r).unwrap()).unwrap();
            d
        };
        let g = write("g", &img, Vec::new());
        let same = write("same", &img, Vec::new());
        let r = write("r", &other, Vec::new());
        let m = write("m", &other, vec![[0, 0, 1, 1]]);
        let diff = dir.join("diff");
        assert_eq!(compare(&g, &same, &diff).unwrap(), None);
        assert_eq!(compare(&g, &m, &diff).unwrap(), None);
        let d = compare(&g, &r, &diff).unwrap().unwrap();
        assert_eq!((d.snap.as_str(), d.pixels), ("open", 1));
        assert!(diff.join("xor.png").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
