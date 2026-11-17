//! `r9 hostcheck`: the host checks of plan 11.9.
//!
//! The command boots the 9h kernel with its screen in a window of the host and with the host
//! clipboard, as a user runs it. In the console window of `rio`, it starts the probe of
//! `guest/probe` in a window that covers the screen. The probe reads `/dev/kbd` and
//! `/dev/mouse` of its window and writes a line for each message to the kernel console. Then a
//! driver gives input to the window of the host, and the command compares the lines of the probe
//! with the input.
//!
//! The drivers:
//!
//! - `x11` gives the input with `XTest` through `xdotool`, and uses the clipboard with `xclip`.
//!   It needs `DISPLAY`, and it is the check of the X11 back end on Linux;
//! - `wayland` gives the input with a virtual keyboard and a virtual pointer through `9h vinput`,
//!   finds the window with `swaymsg`, and uses the clipboard with `wl-copy` and `wl-paste`. It
//!   needs `WAYLAND_DISPLAY` and sway, and it is the check of the Wayland back end on Linux;
//! - `cocoa` gives the input as events of the window system of macOS, with the key codes of
//!   the US layout, through the `post` step of the control socket. The window of the monitor does
//!   each event as an event of a person: the event goes from `NSApplication` through the window
//!   to the view, and the view makes the input of the machine. The clipboard uses `pbcopy` and
//!   `pbpaste`. It is the check of the `AppKit` back end on macOS. The events do not go through
//!   the window server, because that needs the permission of Accessibility for the process;
//! - `control` gives the input on the control socket of the monitor. It does not go through the
//!   window of the host, so it checks only the probe, the guest and this command. It uses the
//!   clipboard with `pbcopy` and `pbpaste` on macOS, and on Linux with `wl-copy` and
//!   `wl-paste` when `WAYLAND_DISPLAY` is set and with `xclip` when it is not.
//!
//! The checks, in order:
//!
//! - `keys`: each key of the US layout gives its rune;
//! - `shift`: each key with Shift gives the rune of Shift;
//! - `modifier change`: Shift down, A down, Shift up and A up give one `A`;
//! - `control`: Control with V, C and D gives the control runes 0x16, 0x03 and 0x04;
//! - `key ups`: each key that went down went up, and at the end no key is down;
//! - `pointer`: button 1 at points near the four corners of the screen and at its centre gives
//!   the point;
//! - `buttons`: buttons 2 and 3 give the bits 2 and 4 of `/dev/mouse`;
//! - `wheel`: the wheel up and down give the bits 8 and 16;
//! - `snarf in`: a text on the host clipboard is the text of `/dev/snarf` in `rio`;
//! - `snarf out`: a text that the probe writes to `/dev/snarf` goes to the host clipboard;
//! - `audio`: the samples that the probe writes to `/dev/audio` are in the WAV file of the
//!   monitor, in order and with no change. The monitor writes the file in place of the audio
//!   output of the host, so the check makes no sound. It checks the audio driver of the guest,
//!   the device of the monitor and its timing, but not the audio output of the host.
//!
//! The command keeps the text on the host clipboard before the checks, and puts it back after
//! them. The output directory has `console.log`, the logs of the monitor and `hostcheck.json`,
//! with the result of each check.

use crate::gui;
use crate::h9boot;
use serde::Serialize;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The source of the probe.
pub const PROBE_C: &str = include_str!("../../../guest/probe/probe.c");

/// The run file of the probe, which builds and runs it in its window.
pub const RUN_RC: &str = include_str!("../../../guest/probe/run.rc");

/// The text before each line of the probe on the console.
const MARK: &str = "R9-PROBE ";

/// The text that the command puts on the host clipboard.
pub const HOST_SNARF: &str = "r9-host-snarf";

/// The text that the probe writes to `/dev/snarf`.
pub const PROBE_SNARF: &str = "r9-probe-snarf";

/// The rc file in the guest that the window of the probe runs.
const LAUNCH_FILE: &str = "/tmp/r9probe.rc";

/// The distance of the points of the `pointer` check from the edges of the screen. The border
/// of the window of `rio` is 4 pixels.
const INSET: u32 = 20;

/// The time that the probe can take to build and start.
const START_SECS: u32 = 110;

/// The time that the probe can take to answer an input.
const ANSWER_SECS: u32 = 30;

/// The keys of the US layout that give a rune: the X keysym name of the key, the rune without
/// Shift and the rune with Shift.
const PUNCTUATION: &[(&str, char, char)] = &[
    ("grave", '`', '~'),
    ("1", '1', '!'),
    ("2", '2', '@'),
    ("3", '3', '#'),
    ("4", '4', '$'),
    ("5", '5', '%'),
    ("6", '6', '^'),
    ("7", '7', '&'),
    ("8", '8', '*'),
    ("9", '9', '('),
    ("0", '0', ')'),
    ("minus", '-', '_'),
    ("equal", '=', '+'),
    ("bracketleft", '[', '{'),
    ("bracketright", ']', '}'),
    ("backslash", '\\', '|'),
    ("semicolon", ';', ':'),
    ("apostrophe", '\'', '"'),
    ("comma", ',', '<'),
    ("period", '.', '>'),
    ("slash", '/', '?'),
    ("space", ' ', ' '),
];

/// One key: its X keysym name, its rune and its rune with Shift.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Key {
    /// The X keysym name.
    pub sym: String,
    /// The rune without Shift.
    pub plain: char,
    /// The rune with Shift.
    pub shifted: char,
}

/// The keys of the checks `keys` and `shift`: the letters, then the other keys.
#[must_use]
pub fn keys() -> Vec<Key> {
    let letters = ('a'..='z').map(|c| Key {
        sym: c.to_string(),
        plain: c,
        shifted: c.to_ascii_uppercase(),
    });
    let other = PUNCTUATION.iter().map(|&(s, p, q)| Key {
        sym: s.to_string(),
        plain: p,
        shifted: q,
    });
    letters.chain(other).collect()
}

/// The rune of the keysym `sym`, with Shift when `shift` is set.
fn rune_of(sym: &str, shift: bool) -> Option<char> {
    keys()
        .into_iter()
        .find(|k| k.sym == sym)
        .map(|k| if shift { k.shifted } else { k.plain })
}

/// One step of the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Push and release the key `sym`, with Shift or Control down when set.
    Tap {
        /// The X keysym name of the key.
        sym: String,
        /// Shift is down.
        shift: bool,
        /// Control is down.
        ctrl: bool,
    },
    /// Push the key `sym`. `Shift` is the left Shift key.
    Down(String),
    /// Release the key `sym`.
    Up(String),
    /// Move the pointer to the point of the screen, and push and release the X button: 1 to 3,
    /// or 4 and 5 for the wheel up and down.
    Click(u32, u32, u8),
    /// Put the text on the host clipboard.
    Copy(String),
    /// Wait until the probe writes the text, which comes after [`MARK`].
    Wait(String),
    /// Read the host clipboard, for the check `snarf out`.
    Paste,
}

/// A change of the buttons: X, Y and the buttons.
pub type Change = (i64, i64, u32);

/// The input of the checks and the output that the probe must give for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    /// The steps of the input.
    pub acts: Vec<Act>,
    /// For each check of the keyboard, the runes of the `c` messages, in order.
    pub typed: Vec<(&'static str, Vec<u32>)>,
    /// For each check of the mouse, the changes of the buttons: X, Y and the buttons.
    pub mouse: Vec<(&'static str, Vec<Change>)>,
}

/// The input of the checks for a screen of `w` by `h` pixels.
#[must_use]
pub fn plan(w: u32, h: u32) -> Plan {
    let mut acts = Vec::new();
    let mut typed = Vec::new();
    let tap = |sym: &str, shift: bool, ctrl: bool| Act::Tap {
        sym: sym.to_string(),
        shift,
        ctrl,
    };
    let all = keys();
    for shift in [false, true] {
        let mut runes = Vec::new();
        for k in &all {
            acts.push(tap(&k.sym, shift, false));
            runes.push(u32::from(if shift { k.shifted } else { k.plain }));
        }
        typed.push((if shift { "shift" } else { "keys" }, runes));
    }
    acts.extend([
        Act::Down("Shift".into()),
        Act::Down("a".into()),
        Act::Up("Shift".into()),
        Act::Up("a".into()),
    ]);
    typed.push(("modifier change", vec![u32::from('A')]));

    let mut mouse = Vec::new();
    let (cx, cy) = (w / 2, h / 2);
    let points = [
        (INSET, INSET),
        (w - 1 - INSET, INSET),
        (INSET, h - 1 - INSET),
        (w - 1 - INSET, h - 1 - INSET),
        (cx, cy),
    ];
    let mut clicks = |name, list: &[(u32, u32, u8)]| {
        let mut want = Vec::new();
        for &(x, y, b) in list {
            acts.push(Act::Click(x, y, b));
            let bit = match b {
                1 => 1,
                2 => 2,
                3 => 4,
                4 => 8,
                _ => 16,
            };
            want.push((i64::from(x), i64::from(y), bit));
            want.push((i64::from(x), i64::from(y), 0));
        }
        mouse.push((name, want));
    };
    let ones: Vec<(u32, u32, u8)> = points.iter().map(|&(x, y)| (x, y, 1)).collect();
    clicks("pointer", &ones);
    clicks("buttons", &[(cx, cy, 2), (cx, cy, 3)]);
    clicks("wheel", &[(cx, cy, 4), (cx, cy, 5)]);

    acts.extend([
        Act::Copy(HOST_SNARF.into()),
        tap("v", false, true),
        Act::Wait("snarf ".into()),
        tap("c", false, true),
        Act::Wait("snarfed".into()),
        Act::Paste,
        tap("d", false, true),
        Act::Wait("end".into()),
    ]);
    typed.push(("control", vec![0x16, 0x03, 0x04]));
    Plan { acts, typed, mouse }
}

/// One line of the probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The probe opened its files.
    Ready,
    /// A message of `/dev/kbd`: its type and its runes.
    Kbd(char, Vec<u32>),
    /// A change of the buttons: X, Y and the buttons.
    Mouse(i64, i64, u32),
    /// The bytes of `/dev/snarf`.
    Snarf(Vec<u8>),
    /// The probe wrote [`PROBE_SNARF`] to `/dev/snarf`.
    Snarfed,
    /// The probe wrote the bytes of the audio pattern to `/dev/audio`.
    Audio(usize),
    /// The probe stops.
    End,
    /// The probe failed.
    Error(String),
}

/// The bytes of the hex text `s`, or None when it is not hex.
fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

/// The event of the text after [`MARK`] on a line, or None when the text is not one.
fn event(text: &str) -> Option<Event> {
    let mut words = text.split_whitespace();
    let e = match words.next()? {
        "ready" => Event::Ready,
        "end" => Event::End,
        "snarfed" => Event::Snarfed,
        "audio" => Event::Audio(words.next()?.parse().ok()?),
        "snarf" => Event::Snarf(unhex(words.next().unwrap_or(""))?),
        "error" => Event::Error(words.collect::<Vec<_>>().join(" ")),
        "kbd" => {
            let t = words.next()?.chars().next()?;
            let runes = words
                .map(|r| u32::from_str_radix(r, 16).ok())
                .collect::<Option<Vec<_>>>()?;
            Event::Kbd(t, runes)
        }
        "mouse" => {
            let mut num = || words.next()?.parse::<i64>().ok();
            let (x, y, buttons) = (num()?, num()?, num()?);
            Event::Mouse(x, y, u32::try_from(buttons).ok()?)
        }
        _ => return None,
    };
    Some(e)
}

/// The lines of the probe in the console output `console`, in order. A line of the console can
/// have other text before the mark.
#[must_use]
pub fn events(console: &str) -> Vec<Event> {
    console
        .lines()
        .filter_map(|l| {
            let at = l.find(MARK)? + MARK.len();
            event(l[at..].trim_end_matches('\r'))
        })
        .collect()
}

/// The result of one check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// The name of the check.
    pub name: String,
    /// True when the check passed.
    pub passed: bool,
    /// What was wrong, or an empty string.
    pub detail: String,
}

impl Check {
    fn new(name: &str, problem: Option<String>) -> Check {
        Check {
            name: name.to_string(),
            passed: problem.is_none(),
            detail: problem.unwrap_or_default(),
        }
    }
}

/// The first difference of `got` from `want`, in words, or None when they are equal.
fn difference<T: PartialEq + std::fmt::Debug>(
    got: &[T],
    want: &[T],
    show: impl Fn(&T) -> String,
) -> Option<String> {
    if got == want {
        return None;
    }
    let i = got.iter().zip(want).take_while(|(a, b)| a == b).count();
    let item = |v: &[T]| v.get(i).map_or_else(|| "nothing".to_string(), &show);
    Some(format!(
        "item {} of {} is {}, and it must be {}",
        i + 1,
        want.len(),
        item(got),
        item(want)
    ))
}

fn rune(r: u32) -> String {
    match char::from_u32(r) {
        Some(c) if c.is_ascii_graphic() => format!("{c:?}"),
        _ => format!("U+{r:04X}"),
    }
}

/// Split `got` into parts with the lengths of the parts of `want`. The last part has the rest.
fn split<'a, T, W>(got: &'a [T], want: &[(&str, Vec<W>)]) -> Vec<&'a [T]> {
    let mut at = 0;
    let mut parts = Vec::new();
    for (i, (_, w)) in want.iter().enumerate() {
        let end = if i + 1 == want.len() {
            got.len()
        } else {
            (at + w.len()).min(got.len())
        };
        parts.push(&got[at.min(end)..end]);
        at = end;
    }
    parts
}

/// The number of 16-bit stereo frames of the audio pattern, as `Audioframes` of the probe.
const AUDIO_FRAMES: u16 = 8192;

/// The bytes that the probe writes to `/dev/audio`: sample `k` is `k*7919+12345` in 16 bits,
/// little endian.
#[must_use]
pub fn audio_pattern() -> Vec<u8> {
    (0..2 * AUDIO_FRAMES)
        .flat_map(|k| k.wrapping_mul(7919).wrapping_add(12345).to_le_bytes())
        .collect()
}

/// The size of the header of the WAV file of the monitor.
const WAV_HEADER: usize = 44;

/// The problem of the audio check, or None when the samples of the probe are in the WAV file
/// `wav`.
fn audio_problem(events: &[Event], wav: Option<&[u8]>) -> Option<String> {
    let want = audio_pattern();
    let wrote = events.iter().find_map(|e| match e {
        Event::Audio(n) => Some(*n),
        _ => None,
    });
    match wrote {
        None => return Some("the probe did not write to /dev/audio".into()),
        Some(n) if n != want.len() => {
            return Some(format!(
                "the probe wrote {n} bytes to /dev/audio, and it must write {}",
                want.len()
            ));
        }
        Some(_) => {}
    }
    let Some(wav) = wav else {
        return Some("the monitor wrote no WAV file".into());
    };
    if wav.len() < WAV_HEADER || &wav[..4] != b"RIFF" || &wav[8..12] != b"WAVE" {
        return Some(format!("the WAV file has no header ({} bytes)", wav.len()));
    }
    let data = &wav[WAV_HEADER..];
    if data.windows(want.len()).any(|w| w == want.as_slice()) {
        return None;
    }
    let head = &want[..64];
    Some(match data.windows(head.len()).position(|w| w == head) {
        Some(at) => {
            let same = data[at..]
                .iter()
                .zip(&want)
                .take_while(|(a, b)| a == b)
                .count();
            format!(
                "the WAV file has the first {same} of the {} bytes of the pattern",
                want.len()
            )
        }
        None => format!(
            "the WAV file has {} bytes of samples, and not the pattern of the probe",
            data.len()
        ),
    })
}

/// The checks of the events of the probe for the plan. `pasted` is the host clipboard after the
/// probe wrote to `/dev/snarf`, and `wav` is the WAV file of the monitor.
#[must_use]
pub fn evaluate(
    events: &[Event],
    plan: &Plan,
    pasted: Option<&str>,
    wav: Option<&[u8]>,
) -> Vec<Check> {
    let mut checks = Vec::new();
    let typed: Vec<u32> = events
        .iter()
        .filter_map(|e| match e {
            Event::Kbd('c', r) => r.first().copied(),
            _ => None,
        })
        .collect();
    for ((name, want), got) in plan.typed.iter().zip(split(&typed, &plan.typed)) {
        checks.push(Check::new(name, difference(got, want, |r| rune(*r))));
    }

    // Each k or K message has the keys that are down after it.
    let mut down = BTreeSet::new();
    let mut counts: BTreeMap<u32, (u32, u32)> = BTreeMap::new();
    for e in events {
        if let Event::Kbd('k' | 'K', r) = e {
            let now: BTreeSet<u32> = r.iter().copied().collect();
            for k in now.difference(&down) {
                counts.entry(*k).or_default().0 += 1;
            }
            for k in down.difference(&now) {
                counts.entry(*k).or_default().1 += 1;
            }
            down = now;
        }
    }
    let unmatched: Vec<String> = counts
        .iter()
        .filter(|(_, (d, u))| d != u)
        .map(|(k, (d, u))| format!("{} down {d} times and up {u} times", rune(*k)))
        .collect();
    let problem = if counts.is_empty() {
        Some("the probe got no key down".to_string())
    } else if !unmatched.is_empty() {
        Some(unmatched.join(", "))
    } else if !down.is_empty() {
        let d: Vec<String> = down.iter().map(|r| rune(*r)).collect();
        Some(format!("at the end, {} is down", d.join(" ")))
    } else {
        None
    };
    checks.push(Check::new("key ups", problem));

    let mouse: Vec<(i64, i64, u32)> = events
        .iter()
        .filter_map(|e| match e {
            Event::Mouse(x, y, b) => Some((*x, *y, *b)),
            _ => None,
        })
        .collect();
    for ((name, want), got) in plan.mouse.iter().zip(split(&mouse, &plan.mouse)) {
        let show = |m: &(i64, i64, u32)| format!("buttons {} at {},{}", m.2, m.0, m.1);
        checks.push(Check::new(name, difference(got, want, show)));
    }

    let snarf = events.iter().find_map(|e| match e {
        Event::Snarf(b) => Some(String::from_utf8_lossy(b).to_string()),
        _ => None,
    });
    checks.push(Check::new(
        "snarf in",
        match snarf {
            Some(s) if s == HOST_SNARF => None,
            Some(s) => Some(format!(
                "/dev/snarf has {s:?}, and it must have {HOST_SNARF:?}"
            )),
            None => Some("the probe did not read /dev/snarf".into()),
        },
    ));
    checks.push(Check::new(
        "snarf out",
        match pasted {
            Some(s) if s == PROBE_SNARF => None,
            Some(s) => Some(format!(
                "the host clipboard has {s:?}, and it must have {PROBE_SNARF:?}"
            )),
            None => Some("the host clipboard was not read".into()),
        },
    ));
    checks.push(Check::new("audio", audio_problem(events, wav)));
    checks
}

/// A way to give input to the 9h machine.
pub trait Driver {
    /// The name of the driver.
    fn name(&self) -> &'static str;
    /// Find the window of the machine and give it the focus.
    ///
    /// # Errors
    ///
    /// When the window is not found.
    fn start(&mut self, control: &Path) -> Result<(), String>;
    /// Do one step of the input. [`Act::Wait`] does not come here.
    ///
    /// # Errors
    ///
    /// When a program or the monitor fails.
    fn act(&mut self, act: &Act, control: &Path) -> Result<(), String>;
}

/// Run a program and return its standard output.
fn program(name: &str, args: &[&str], input: Option<&str>) -> Result<String, String> {
    let mut c = Command::new(name);
    c.args(args).stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    // xclip -i stays to serve the selection, with the output of its parent. Thus the output of a
    // program that reads input is not kept, or the read would wait for it.
    if input.is_some() {
        c.stdout(Stdio::null()).stderr(Stdio::null());
    } else {
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = c.spawn().map_err(|e| format!("cannot run {name}: {e}"))?;
    if let (Some(text), Some(mut w)) = (input, child.stdin.take()) {
        w.write_all(text.as_bytes())
            .map_err(|e| format!("{name}: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{name}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{name} {}: {}: {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// The programs of the host clipboard: the program that writes it and the program that reads it.
fn clipboard() -> (Vec<&'static str>, Vec<&'static str>) {
    if cfg!(target_os = "macos") {
        (vec!["pbcopy"], vec!["pbpaste"])
    } else if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty()) {
        (vec!["wl-copy"], vec!["wl-paste", "--no-newline"])
    } else {
        (
            vec!["xclip", "-selection", "clipboard", "-i"],
            vec!["xclip", "-selection", "clipboard", "-o"],
        )
    }
}

/// Put `text` on the host clipboard.
fn copy(text: &str) -> Result<(), String> {
    let (w, _) = clipboard();
    program(w[0], &w[1..], Some(text)).map(|_| ())
}

/// The text of the host clipboard.
fn paste() -> Result<String, String> {
    let (_, r) = clipboard();
    program(r[0], &r[1..], None)
}

fn pause(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

/// The driver of an X11 display, with `xdotool` and `xclip`.
#[derive(Debug, Default)]
pub struct X11 {
    window: String,
}

impl X11 {
    fn xdotool(args: &[&str]) -> Result<String, String> {
        program("xdotool", args, None)
    }

    fn sym(sym: &str) -> &str {
        if sym == "Shift" { "Shift_L" } else { sym }
    }
}

impl Driver for X11 {
    fn name(&self) -> &'static str {
        "x11"
    }

    fn start(&mut self, _: &Path) -> Result<(), String> {
        let start = Instant::now();
        loop {
            if let Ok(out) = X11::xdotool(&["search", "--name", "^9h: "])
                && let Some(w) = out.lines().next()
            {
                self.window = w.trim().to_string();
                break;
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err("xdotool found no window with a name that starts with 9h:".into());
            }
            pause(500);
        }
        // With no window manager, windowfocus sets the focus, and windowactivate fails. With a
        // window manager, windowactivate raises the window.
        let _ = X11::xdotool(&["windowactivate", "--sync", &self.window]);
        let _ = X11::xdotool(&["windowfocus", "--sync", &self.window]);
        X11::xdotool(&["mousemove", "--window", &self.window, "300", "300"])?;
        pause(500);
        Ok(())
    }

    fn act(&mut self, act: &Act, _: &Path) -> Result<(), String> {
        match act {
            Act::Tap { sym, shift, ctrl } => {
                let mut k = String::new();
                if *ctrl {
                    k.push_str("ctrl+");
                }
                if *shift {
                    k.push_str("shift+");
                }
                k.push_str(sym);
                X11::xdotool(&["key", &k])?;
            }
            Act::Down(sym) => {
                X11::xdotool(&["keydown", X11::sym(sym)])?;
            }
            Act::Up(sym) => {
                X11::xdotool(&["keyup", X11::sym(sym)])?;
            }
            Act::Click(x, y, b) => {
                let (x, y) = (x.to_string(), y.to_string());
                X11::xdotool(&["mousemove", "--window", &self.window, &x, &y])?;
                pause(100);
                X11::xdotool(&["click", &b.to_string()])?;
            }
            Act::Copy(text) => copy(text)?,
            Act::Wait(_) | Act::Paste => {}
        }
        pause(80);
        Ok(())
    }
}

/// The Linux key code of the key with the X keysym name `sym`, on a US keyboard.
#[must_use]
pub fn key_code(sym: &str) -> Option<u32> {
    const ROWS: &[(u32, &str)] = &[
        (16, "qwertyuiop"),
        (30, "asdfghjkl"),
        (44, "zxcvbnm"),
        (2, "1234567890"),
    ];
    let mut chars = sym.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        for &(first, row) in ROWS {
            if let Some(i) = row.find(c) {
                return u32::try_from(i).ok().map(|i| first + i);
            }
        }
    }
    Some(match sym {
        "Escape" => 1,
        "minus" => 12,
        "equal" => 13,
        "BackSpace" => 14,
        "Tab" => 15,
        "bracketleft" => 26,
        "bracketright" => 27,
        "Return" => 28,
        "Control" => 29,
        "semicolon" => 39,
        "apostrophe" => 40,
        "grave" => 41,
        "Shift" => 42,
        "backslash" => 43,
        "comma" => 51,
        "period" => 52,
        "slash" => 53,
        "space" => 57,
        _ => return None,
    })
}

/// The modifier bits of the Wayland keyboard: Shift and Control.
const MOD_SHIFT: u32 = 1;
const MOD_CONTROL: u32 = 4;

/// The Linux button codes of the left, middle and right buttons.
const BUTTON_CODES: [u32; 3] = [0x110, 0x112, 0x111];

/// The driver of a Wayland session of sway. `9h vinput` gives the input with a virtual
/// keyboard and a virtual pointer, `swaymsg` finds the window and gives it the focus, and
/// `wl-copy` and `wl-paste` use the clipboard.
#[derive(Debug)]
pub struct Wayland {
    monitor: std::path::PathBuf,
    child: Option<std::process::Child>,
    answers: Option<std::io::BufReader<std::process::ChildStdout>>,
    /// The corner of the window in the layout.
    corner: (i64, i64),
    /// The size of the layout.
    layout: (i64, i64),
    mods: u32,
}

impl Wayland {
    /// The driver that runs `vinput` of the monitor `monitor`.
    #[must_use]
    pub fn new(monitor: &Path) -> Wayland {
        Wayland {
            monitor: monitor.to_path_buf(),
            child: None,
            answers: None,
            corner: (0, 0),
            layout: (0, 0),
            mods: 0,
        }
    }

    /// Give one step to `vinput`, and wait for its answer.
    fn step(&mut self, line: &str) -> Result<(), String> {
        use std::io::BufRead;
        let (Some(child), Some(answers)) = (&mut self.child, &mut self.answers) else {
            return Err("vinput does not run".into());
        };
        let stdin = child.stdin.as_mut().ok_or("no input for vinput")?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|e| format!("vinput: {e}"))?;
        let mut answer = String::new();
        answers
            .read_line(&mut answer)
            .map_err(|e| format!("vinput: {e}"))?;
        match answer.trim() {
            "ok" => Ok(()),
            "" => Err("vinput stopped".into()),
            a => Err(format!("vinput: {line}: {a}")),
        }
    }

    fn key(&mut self, code: u32, down: bool) -> Result<(), String> {
        self.step(&format!("key {code} {}", u8::from(down)))
    }

    /// Push or release a modifier key, and give the new modifiers.
    fn modifier(&mut self, code: u32, bit: u32, down: bool) -> Result<(), String> {
        self.key(code, down)?;
        if down {
            self.mods |= bit;
        } else {
            self.mods &= !bit;
        }
        self.step(&format!("mods {}", self.mods))
    }

    fn code(sym: &str) -> Result<u32, String> {
        key_code(sym).ok_or_else(|| format!("no key code for {sym}"))
    }
}

/// The corner of the content of the window with the app ID `9h` in the tree of sway, and the
/// size of the layout. A node has `rect` in the layout, and `window_rect` in the node.
#[must_use]
pub fn sway_window(tree: &serde_json::Value) -> Option<((i64, i64), (i64, i64))> {
    fn find(n: &serde_json::Value) -> Option<(i64, i64)> {
        if n["app_id"] == "9h" {
            let at = |r: &serde_json::Value, k: &str| r[k].as_i64().unwrap_or(0);
            let (r, w) = (&n["rect"], &n["window_rect"]);
            return Some((at(r, "x") + at(w, "x"), at(r, "y") + at(w, "y")));
        }
        ["nodes", "floating_nodes"]
            .iter()
            .filter_map(|k| n[*k].as_array())
            .flatten()
            .find_map(find)
    }
    let r = &tree["rect"];
    let layout = (r["width"].as_i64()?, r["height"].as_i64()?);
    find(tree).map(|corner| (corner, layout))
}

impl Driver for Wayland {
    fn name(&self) -> &'static str {
        "wayland"
    }

    fn start(&mut self, _: &Path) -> Result<(), String> {
        let start = Instant::now();
        loop {
            let tree = program("swaymsg", &["-t", "get_tree", "-r"], None)
                .ok()
                .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok());
            if let Some((corner, layout)) = tree.as_ref().and_then(sway_window) {
                (self.corner, self.layout) = (corner, layout);
                break;
            }
            if start.elapsed() > Duration::from_secs(30) {
                return Err("swaymsg found no window with the app ID 9h".into());
            }
            pause(500);
        }
        let mut child = Command::new(&self.monitor)
            .arg("vinput")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run {} vinput: {e}", self.monitor.display()))?;
        let out = child.stdout.take().ok_or("no output of vinput")?;
        let mut answers = std::io::BufReader::new(out);
        let mut ready = String::new();
        std::io::BufRead::read_line(&mut answers, &mut ready)
            .map_err(|e| format!("vinput: {e}"))?;
        self.child = Some(child);
        self.answers = Some(answers);
        if ready.trim() != "ok" {
            return Err(format!("vinput did not start: {:?}", ready.trim()));
        }
        // The keyboard came after the window, so the window gets the focus again.
        program("swaymsg", &["[app_id=\"9h\"]", "focus"], None)?;
        let (x, y) = (self.corner.0 + 300, self.corner.1 + 300);
        self.step(&format!("move {x} {y} {} {}", self.layout.0, self.layout.1))?;
        pause(500);
        Ok(())
    }

    fn act(&mut self, act: &Act, _: &Path) -> Result<(), String> {
        match act {
            Act::Tap { sym, shift, ctrl } => {
                let code = Wayland::code(sym)?;
                if *ctrl {
                    self.modifier(29, MOD_CONTROL, true)?;
                }
                if *shift {
                    self.modifier(42, MOD_SHIFT, true)?;
                }
                self.key(code, true)?;
                self.key(code, false)?;
                if *shift {
                    self.modifier(42, MOD_SHIFT, false)?;
                }
                if *ctrl {
                    self.modifier(29, MOD_CONTROL, false)?;
                }
            }
            Act::Down(sym) if sym == "Shift" => self.modifier(42, MOD_SHIFT, true)?,
            Act::Up(sym) if sym == "Shift" => self.modifier(42, MOD_SHIFT, false)?,
            Act::Down(sym) => self.key(Wayland::code(sym)?, true)?,
            Act::Up(sym) => self.key(Wayland::code(sym)?, false)?,
            Act::Click(x, y, b) => {
                let (x, y) = (self.corner.0 + i64::from(*x), self.corner.1 + i64::from(*y));
                self.step(&format!("move {x} {y} {} {}", self.layout.0, self.layout.1))?;
                pause(100);
                match b {
                    1..=3 => {
                        let code = BUTTON_CODES[usize::from(b - 1)];
                        self.step(&format!("button {code} 1"))?;
                        pause(50);
                        self.step(&format!("button {code} 0"))?;
                    }
                    4 => self.step("wheel -1")?,
                    _ => self.step("wheel 1")?,
                }
            }
            Act::Copy(text) => copy(text)?,
            Act::Wait(_) | Act::Paste => {}
        }
        pause(80);
        Ok(())
    }
}

impl Drop for Wayland {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            drop(c.stdin.take());
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// The driver of the control socket of the monitor. It checks the probe and this command, and
/// not the window of the host.
#[derive(Debug, Default)]
pub struct Control {
    shift: bool,
}

impl Driver for Control {
    fn name(&self) -> &'static str {
        "control"
    }

    fn start(&mut self, _: &Path) -> Result<(), String> {
        Ok(())
    }

    fn act(&mut self, act: &Act, control: &Path) -> Result<(), String> {
        let send = |r: char| {
            let mut b = [0; 4];
            gui::send(r.encode_utf8(&mut b).as_bytes())
        };
        let lines = match act {
            Act::Tap { sym, shift, ctrl } => {
                let r = rune_of(sym, *shift).ok_or_else(|| format!("no rune for {sym}"))?;
                let r = if *ctrl {
                    char::from(u8::try_from(r).unwrap_or(0) & 0x1f)
                } else {
                    r
                };
                vec![send(r)]
            }
            Act::Down(sym) if sym == "Shift" => {
                self.shift = true;
                Vec::new()
            }
            Act::Up(sym) if sym == "Shift" => {
                self.shift = false;
                Vec::new()
            }
            // The socket types a key down and up at once, so the key goes at its push.
            Act::Down(sym) => {
                let r = rune_of(sym, self.shift).ok_or_else(|| format!("no rune for {sym}"))?;
                vec![send(r)]
            }
            Act::Click(x, y, b) => {
                let bit = match b {
                    1 => 1,
                    2 => 2,
                    3 => 4,
                    4 => 8,
                    _ => 16,
                };
                vec![
                    format!("mouse {x} {y} 0"),
                    format!("mouse {x} {y} {bit}"),
                    format!("mouse {x} {y} 0"),
                ]
            }
            Act::Copy(text) => {
                copy(text)?;
                Vec::new()
            }
            Act::Up(_) | Act::Wait(_) | Act::Paste => Vec::new(),
        };
        gui::give(control, &lines)
    }
}

/// The key code of macOS for the key with the X keysym name `sym`, on the US layout (ANSI).
#[must_use]
pub fn mac_key_code(sym: &str) -> Option<u16> {
    const LETTERS: [u16; 26] = [
        0, 11, 8, 2, 14, 3, 5, 4, 34, 38, 40, 37, 46, 45, 31, 35, 12, 15, 1, 17, 32, 9, 13, 7, 16,
        6,
    ];
    const DIGITS: [u16; 10] = [29, 18, 19, 20, 21, 23, 22, 26, 28, 25];
    let mut chars = sym.chars();
    if let (Some(c), None) = (chars.next(), chars.next()) {
        if c.is_ascii_lowercase() {
            return Some(LETTERS[usize::from(c as u8 - b'a')]);
        }
        if c.is_ascii_digit() {
            return Some(DIGITS[usize::from(c as u8 - b'0')]);
        }
    }
    Some(match sym {
        "equal" => 24,
        "minus" => 27,
        "bracketright" => 30,
        "bracketleft" => 33,
        "Return" => 36,
        "apostrophe" => 39,
        "semicolon" => 41,
        "backslash" => 42,
        "comma" => 43,
        "slash" => 44,
        "period" => 47,
        "Tab" => 48,
        "space" => 49,
        "grave" => 50,
        "BackSpace" => 51,
        "Escape" => 53,
        "Shift" => MAC_SHIFT,
        "Control" => MAC_CONTROL,
        _ => return None,
    })
}

/// The key codes of macOS of the left Shift key and the left Control key.
const MAC_SHIFT: u16 = 56;
const MAC_CONTROL: u16 = 59;

/// The driver of the window of macOS. It gives the input as events of `AppKit` with the `post`
/// step of the control socket, and uses the clipboard with `pbcopy` and `pbpaste`.
#[derive(Debug, Default)]
pub struct Cocoa {
    /// The modifiers that are down, as words of the `post` step.
    mods: Vec<&'static str>,
}

impl Cocoa {
    fn code(sym: &str) -> Result<u16, String> {
        mac_key_code(sym).ok_or_else(|| format!("no key code for {sym}"))
    }

    /// The `post` line that pushes or releases a modifier key.
    fn modifier(&mut self, code: u16, word: &'static str, down: bool) -> String {
        self.mods.retain(|&m| m != word);
        if down {
            self.mods.push(word);
        }
        format!("post flags {code} {}", self.mods.join(" "))
            .trim_end()
            .to_string()
    }

    /// The `post` line of a key down or up, with the modifiers that are down.
    fn key(&self, code: u16, down: bool) -> String {
        let dir = if down { "down" } else { "up" };
        format!("post key {code} {dir} {}", self.mods.join(" "))
            .trim_end()
            .to_string()
    }
}

impl Driver for Cocoa {
    fn name(&self) -> &'static str {
        "cocoa"
    }

    fn start(&mut self, control: &Path) -> Result<(), String> {
        if !cfg!(target_os = "macos") {
            return Err("the cocoa driver needs macOS".into());
        }
        let (w, h) = gui::SCREEN;
        gui::give(control, &[format!("post move {} {}", w / 2, h / 2)])
    }

    fn act(&mut self, act: &Act, control: &Path) -> Result<(), String> {
        let lines = match act {
            Act::Tap { sym, shift, ctrl } => {
                let code = Cocoa::code(sym)?;
                let mut lines = Vec::new();
                if *ctrl {
                    lines.push(self.modifier(MAC_CONTROL, "control", true));
                }
                if *shift {
                    lines.push(self.modifier(MAC_SHIFT, "shift", true));
                }
                lines.push(self.key(code, true));
                lines.push(self.key(code, false));
                if *shift {
                    lines.push(self.modifier(MAC_SHIFT, "shift", false));
                }
                if *ctrl {
                    lines.push(self.modifier(MAC_CONTROL, "control", false));
                }
                lines
            }
            Act::Down(sym) if sym == "Shift" => vec![self.modifier(MAC_SHIFT, "shift", true)],
            Act::Up(sym) if sym == "Shift" => vec![self.modifier(MAC_SHIFT, "shift", false)],
            Act::Down(sym) => vec![self.key(Cocoa::code(sym)?, true)],
            Act::Up(sym) => vec![self.key(Cocoa::code(sym)?, false)],
            Act::Click(x, y, b) => {
                let mut lines = vec![format!("post move {x} {y}")];
                match b {
                    // The button numbers of macOS: 0 left, 1 right and 2 middle.
                    1..=3 => {
                        let n = [0, 2, 1][usize::from(b - 1)];
                        lines.push(format!("post down {x} {y} {n}"));
                        lines.push(format!("post up {x} {y} {n}"));
                    }
                    4 => lines.push(format!("post wheel {x} {y} up")),
                    _ => lines.push(format!("post wheel {x} {y} down")),
                }
                lines
            }
            Act::Copy(text) => {
                copy(text)?;
                Vec::new()
            }
            Act::Wait(_) | Act::Paste => Vec::new(),
        };
        gui::give(control, &lines)
    }
}

/// The lines for the control socket that start the probe in a window that covers the screen,
/// from the console window of `rio`.
#[must_use]
pub fn launch_lines(w: u32, h: u32) -> Vec<String> {
    let line = format!(
        "echo 'mount -nc ''#Z/hostfs'' /mnt/term; rc /mnt/term/run.rc' > {LAUNCH_FILE}; window -r 0 0 {w} {h} rc {LAUNCH_FILE}\n"
    );
    gui::keys(line.as_bytes()).map(gui::send).collect()
}

/// The result of `r9 hostcheck`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
    /// The driver.
    pub host: String,
    /// The checks, in order.
    pub checks: Vec<Check>,
    /// True when each check passed.
    pub passed: bool,
    /// Why the run failed before the checks, or why the boot failed.
    pub failure: Option<String>,
}

/// Start the probe and give the input. Return the host clipboard of the `snarf out` check.
fn drive(driver: &mut dyn Driver, plan: &Plan, control: &Path) -> Result<Option<String>, String> {
    let (w, h) = gui::SCREEN;
    gui::give(control, &launch_lines(w, h)).map_err(|e| format!("the launch: {e}"))?;
    gui::give(
        control,
        &[
            format!("timeout {START_SECS}"),
            format!("wait \"{MARK}ready\""),
            format!("timeout {ANSWER_SECS}"),
            "pause 1".into(),
        ],
    )
    .map_err(|e| format!("the probe did not start: {e}"))?;
    driver.start(control)?;
    let mut pasted = None;
    for a in &plan.acts {
        match a {
            Act::Wait(t) => gui::give(control, &[format!("wait \"{MARK}{t}\"")])
                .map_err(|e| format!("the probe did not write {MARK}{t}: {e}"))?,
            Act::Paste => {
                // The monitor runs the copy program of the host after the probe closed
                // /dev/snarf, and the program can take a moment to own the clipboard.
                pause(1000);
                pasted = Some(paste()?);
            }
            _ => driver.act(a, control)?,
        }
    }
    // rio closes the window of the probe when the probe stops, and the console window gets the
    // keys of the halt after this click.
    let (x, y) = gui::CONSOLE_POINT;
    gui::give(control, &[format!("click {x} {y}"), "pause 1".into()])?;
    Ok(pasted)
}

/// Run the host checks with `driver` on the 9h machine of `req`. The output goes to
/// `req.work`.
///
/// # Errors
///
/// When the work directory cannot be written, or the machine does not start.
pub fn run(req: &h9boot::Request, driver: &mut dyn Driver) -> Result<Record, String> {
    let mut req = req.clone();
    req.commands = Vec::new();
    req.window = true;
    let work = req.work.clone();
    let hostfs = work.join("hostfs");
    std::fs::create_dir_all(&hostfs).map_err(|e| format!("{}: {e}", hostfs.display()))?;
    for (f, text) in [("probe.c", PROBE_C), ("run.rc", RUN_RC)] {
        let p = hostfs.join(f);
        std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    req.hostfs = Some(hostfs);
    req.snap = Some(work.join("screen.png"));
    let wav_path = work.join("audio.wav");
    let _ = std::fs::remove_file(&wav_path);
    req.audio = Some(wav_path.clone());
    let (w, h) = gui::SCREEN;
    let plan = plan(w, h);
    let saved = paste().ok();
    let mut result = Err("the guest gave no shell prompt".to_string());
    let outcome = h9boot::run_with(&req, |path| {
        result = drive(driver, &plan, path);
        if result.is_err() {
            let _ = gui::give(path, &["regs".into()]);
        }
    });
    if let Some(s) = &saved {
        let _ = copy(s);
    }
    let outcome = outcome?;
    let f = work.join("console.log");
    let console = std::fs::read_to_string(&f).map_err(|e| format!("{}: {e}", f.display()))?;
    let events = events(&console);
    let pasted = result.as_ref().ok().cloned().flatten();
    let wav = std::fs::read(&wav_path).ok();
    let checks = evaluate(&events, &plan, pasted.as_deref(), wav.as_deref());
    let errors: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::Error(t) => Some(t.clone()),
            _ => None,
        })
        .collect();
    let failure = match &result {
        Err(e) => Some(e.clone()),
        Ok(_) if !errors.is_empty() => Some(format!("the probe failed: {}", errors.join("; "))),
        Ok(_) if !outcome.passed => Some("the boot did not pass".into()),
        Ok(_) => None,
    };
    let record = Record {
        host: driver.name().to_string(),
        passed: failure.is_none() && checks.iter().all(|c| c.passed),
        checks,
        failure,
    };
    let json = serde_json::to_string_pretty(&record).map_err(|e| e.to_string())?;
    let f = work.join("hostcheck.json");
    std::fs::write(&f, json + "\n").map_err(|e| format!("{}: {e}", f.display()))?;
    Ok(record)
}

/// The report of a record, one line for each check.
#[must_use]
pub fn report(record: &Record) -> String {
    let mut s = String::new();
    for c in &record.checks {
        if c.passed {
            let _ = writeln!(s, "PASS {}", c.name);
        } else {
            let _ = writeln!(s, "FAIL {}: {}", c.name, c.detail);
        }
    }
    if let Some(f) = &record.failure {
        let _ = writeln!(s, "FAIL the run: {f}");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mac_key_codes_are_those_of_the_us_layout() {
        assert_eq!(mac_key_code("a"), Some(0));
        assert_eq!(mac_key_code("z"), Some(6));
        assert_eq!(mac_key_code("q"), Some(12));
        assert_eq!(mac_key_code("0"), Some(29));
        assert_eq!(mac_key_code("9"), Some(25));
        assert_eq!(mac_key_code("Return"), Some(36));
        assert_eq!(mac_key_code("BackSpace"), Some(51));
        assert_eq!(mac_key_code("F1"), None);
        for k in keys() {
            assert!(mac_key_code(&k.sym).is_some(), "{}", k.sym);
        }
    }

    #[test]
    fn the_cocoa_driver_holds_the_modifiers() {
        let mut c = Cocoa::default();
        assert_eq!(c.modifier(MAC_SHIFT, "shift", true), "post flags 56 shift");
        assert_eq!(c.key(0, true), "post key 0 down shift");
        assert_eq!(c.modifier(MAC_SHIFT, "shift", false), "post flags 56");
        assert_eq!(c.key(0, false), "post key 0 up");
    }

    /// The events of a probe that got the input of the plan as it must.
    fn good(plan: &Plan) -> Vec<Event> {
        let mut e = vec![Event::Ready];
        for (_, runes) in &plan.typed {
            for &r in runes {
                e.push(Event::Kbd('k', vec![r]));
                e.push(Event::Kbd('c', vec![r]));
                e.push(Event::Kbd('K', vec![]));
            }
        }
        for (_, m) in &plan.mouse {
            e.extend(m.iter().map(|&(x, y, b)| Event::Mouse(x, y, b)));
        }
        e.push(Event::Snarf(HOST_SNARF.as_bytes().to_vec()));
        e.push(Event::Snarfed);
        e.push(Event::Audio(audio_pattern().len()));
        e.push(Event::End);
        e
    }

    /// A WAV file of the monitor with silence, the pattern of the probe and silence.
    fn good_wav() -> Vec<u8> {
        let mut w = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        w.resize(WAV_HEADER, 0);
        w.extend([0u8; 400]);
        w.extend(audio_pattern());
        w.extend([0u8; 400]);
        w
    }

    #[test]
    fn the_plan_has_the_keys_and_the_points() {
        let p = plan(1024, 768);
        assert_eq!(keys().len(), 48);
        assert_eq!(p.typed[0].0, "keys");
        assert_eq!(p.typed[0].1.len(), 48);
        assert_eq!(p.typed[1].1[0], u32::from('A'));
        assert_eq!(
            p.mouse[0].1[..4],
            [(20, 20, 1), (20, 20, 0), (1003, 20, 1), (1003, 20, 0)]
        );
        assert_eq!(p.mouse[0].1[8], (512, 384, 1));
        assert_eq!(p.mouse[2].1[2], (512, 384, 16));
        assert!(p.acts.contains(&Act::Click(20, 747, 1)));
        assert_eq!(p.acts.last(), Some(&Act::Wait("end".into())));
    }

    #[test]
    fn the_lines_of_the_probe_are_read() {
        let console = "boot\r\nR9-PROBE ready\r\nxR9-PROBE kbd k f016 41\r\nR9-PROBE kbd c 41\r\nR9-PROBE mouse 20 747 1\r\nR9-PROBE snarf 6869\r\nR9-PROBE snarf\r\nR9-PROBE error kbd no file\r\nR9-PROBE what\r\n";
        assert_eq!(
            events(console),
            vec![
                Event::Ready,
                Event::Kbd('k', vec![0xf016, 0x41]),
                Event::Kbd('c', vec![0x41]),
                Event::Mouse(20, 747, 1),
                Event::Snarf(b"hi".to_vec()),
                Event::Snarf(Vec::new()),
                Event::Error("kbd no file".into()),
            ]
        );
    }

    #[test]
    fn a_good_probe_passes_each_check() {
        let p = plan(1024, 768);
        let checks = evaluate(&good(&p), &p, Some(PROBE_SNARF), Some(&good_wav()));
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "keys",
                "shift",
                "modifier change",
                "control",
                "key ups",
                "pointer",
                "buttons",
                "wheel",
                "snarf in",
                "snarf out",
                "audio"
            ]
        );
        assert!(checks.iter().all(|c| c.passed), "{checks:?}");
    }

    #[test]
    fn a_wrong_rune_and_a_key_that_stays_down_fail() {
        let p = plan(1024, 768);
        let mut e = good(&p);
        let at = e
            .iter()
            .position(|x| *x == Event::Kbd('c', vec![u32::from('A')]))
            .unwrap();
        e[at] = Event::Kbd('c', vec![u32::from('a')]);
        e.push(Event::Kbd('k', vec![0xf016]));
        let checks = evaluate(&e, &p, Some("other"), Some(&good_wav()));
        let failed: Vec<(&str, &str)> = checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| (c.name.as_str(), c.detail.as_str()))
            .collect();
        assert_eq!(
            failed,
            [
                ("shift", "item 1 of 48 is 'a', and it must be 'A'"),
                ("key ups", "U+F016 down 1 times and up 0 times"),
                (
                    "snarf out",
                    "the host clipboard has \"other\", and it must have \"r9-probe-snarf\""
                ),
            ]
        );
    }

    #[test]
    fn the_audio_check_finds_the_pattern_in_the_wav_file() {
        let pattern = audio_pattern();
        assert_eq!(pattern.len(), 4 * usize::from(AUDIO_FRAMES));
        assert_eq!(pattern[..4], [0x39, 0x30, 0x28, 0x4f]);
        let e = [Event::Audio(pattern.len())];
        assert_eq!(audio_problem(&e, Some(&good_wav())), None);
        let mut cut = good_wav();
        cut.truncate(WAV_HEADER + 400 + 1000);
        assert_eq!(
            audio_problem(&e, Some(&cut)).unwrap(),
            "the WAV file has the first 1000 of the 32768 bytes of the pattern"
        );
        assert_eq!(
            audio_problem(&[], Some(&good_wav())).unwrap(),
            "the probe did not write to /dev/audio"
        );
        assert_eq!(
            audio_problem(&e, None).unwrap(),
            "the monitor wrote no WAV file"
        );
        assert_eq!(events("R9-PROBE audio 32768\r\n"), e);
    }

    #[test]
    fn a_lost_click_fails_the_mouse_check() {
        let p = plan(1024, 768);
        let mut e = good(&p);
        let at = e
            .iter()
            .position(|x| *x == Event::Mouse(512, 384, 2))
            .unwrap();
        e.remove(at);
        let checks = evaluate(&e, &p, Some(PROBE_SNARF), Some(&good_wav()));
        let b = checks.iter().find(|c| c.name == "buttons").unwrap();
        assert_eq!(
            b.detail,
            "item 1 of 4 is buttons 0 at 512,384, and it must be buttons 2 at 512,384"
        );
    }

    #[test]
    fn each_key_of_the_plan_has_a_key_code() {
        for k in keys() {
            assert!(key_code(&k.sym).is_some(), "{}", k.sym);
        }
        assert_eq!(key_code("a"), Some(30));
        assert_eq!(key_code("m"), Some(50));
        assert_eq!(key_code("0"), Some(11));
        assert_eq!(key_code("slash"), Some(53));
        assert_eq!(key_code("F13"), None);
    }

    #[test]
    fn the_window_of_9h_is_found_in_the_tree_of_sway() {
        let tree = serde_json::json!({
            "rect": {"x": 0, "y": 0, "width": 1280, "height": 1024},
            "nodes": [{
                "rect": {"x": 0, "y": 0, "width": 1280, "height": 1024},
                "nodes": [],
                "floating_nodes": [{
                    "app_id": "9h",
                    "rect": {"x": 10, "y": 20, "width": 1028, "height": 772},
                    "window_rect": {"x": 2, "y": 2, "width": 1024, "height": 768}
                }]
            }]
        });
        assert_eq!(sway_window(&tree), Some(((12, 22), (1280, 1024))));
        assert_eq!(
            sway_window(&serde_json::json!({"rect": {"width": 1, "height": 1}})),
            None
        );
    }

    #[test]
    fn the_launch_types_one_key_on_each_line() {
        let l = launch_lines(1024, 768);
        assert_eq!(l[0], "send \"e\"");
        assert_eq!(l.last().unwrap(), "send \"\\n\"");
        let typed: String = l
            .iter()
            .map(|s| s.trim_start_matches("send \"").trim_end_matches('"'))
            .collect();
        assert!(typed.contains("window -r 0 0 1024 768 rc /tmp/r9probe.rc"));
        assert!(!typed.contains(MARK));
    }
}
