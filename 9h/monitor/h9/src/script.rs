//! The session scripts of `9h run --script FILE` (plan 10.8 and 12.6). A script drives the
//! console of a session with no person at it, as `r9 9h boot` and `r9 9h smoke` do. Each line is
//! one step:
//! - `wait TEXT` waits until the console writes TEXT after the last match;
//! - `absent TEXT` fails when the console wrote TEXT after the last match, such as the exit line
//!   of a program that must still run;
//! - `send TEXT` types TEXT on the console;
//! - `timeout SECS` sets the time that each later `wait` can take, 60 seconds at the start;
//! - `pause SECS` waits SECS seconds, such as for a program that does not write to the console;
//! - `mouse X Y [BUTTONS]` moves the mouse to the pixel (X, Y) of the screen with BUTTONS down, 0
//!   by default: 1 is the left button, 2 the middle, 4 the right, and 8 and 16 the wheel;
//! - `click X Y [BUTTON]` moves the mouse to (X, Y), and pushes and releases BUTTON, 1 by default;
//! - `quiet [MS [SECS [X0 Y0 X1 Y1]]]` waits until the screen, or the rectangle of it from
//!   (X0, Y0) to (X1, Y1), did not change for MS milliseconds, 500 by default, and fails when that
//!   does not occur in SECS seconds, 30 by default;
//! - `snap FILE [X0 Y0 X1 Y1]` writes the screen, or the rectangle of it from (X0, Y0) to
//!   (X1, Y1), to FILE as a PNG file;
//! - `regs` makes each vCPU write its registers to the standard error of the monitor, for a guest
//!   that stops;
//! - `post EVENT` gives an event of the window system of the host to the window of the machine,
//!   which does it as an event of a person (plan 11.9). Only the window of macOS takes it. EVENT
//!   is `key CODE down|up [MODS]`, `flags CODE [MODS]`, `down X Y BUTTON`, `up X Y BUTTON`,
//!   `move X Y` or `wheel X Y up|down`. CODE is a key code of the host, MODS are the words
//!   `shift`, `control`, `option` and `command`, (X, Y) is a pixel of the screen, and BUTTON is
//!   a button of the host: 0 left, 1 right and 2 middle;
//! - `quit` ends the session, and the script passes.
//!
//! TEXT is the rest of the line with the spaces at its two ends removed, or a string in double quotes with the escapes `\n`, `\r`, `\t`,
//! `\\`, `\"` and `\xHH`. A line that starts with `#` is a comment. The script fails at the first
//! `wait` that times out, at a `quiet` or a `snap` that fails, and when the session ends before
//! the script does. `r9 gui run` makes the steps of a GUI script (plan 12.6) into these steps.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// One step of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Wait for these bytes on the console.
    Wait(Vec<u8>),
    /// Fail when the console wrote these bytes.
    Absent(Vec<u8>),
    /// Type these bytes on the console.
    Send(Vec<u8>),
    /// The time that each later `wait` can take.
    Timeout(Duration),
    /// Wait for this time.
    Pause(Duration),
    /// Move the mouse to (x, y) with these buttons down.
    Mouse(u16, u16, u8),
    /// Wait until the screen, or a rectangle of it, did not change for the first time, for no more
    /// than the second.
    Quiet(Duration, Duration, Option<[u16; 4]>),
    /// Write the screen, or a rectangle of it, to a PNG file.
    Snap(String, Option<[u16; 4]>),
    /// Make each vCPU write its registers to the standard error.
    Regs,
    /// Give an event of the window system of the host to the window.
    Post(h9_gui::HostEvent),
    /// End the session.
    Quit,
}

/// The default time of a `wait`.
pub const TIMEOUT: Duration = Duration::from_secs(60);

/// The time with no change of the screen of a `quiet` with no number.
pub const STILL: Duration = Duration::from_millis(500);

/// The limit of a `quiet` with one number or none.
pub const QUIET_LIMIT: Duration = Duration::from_secs(30);

/// The machine that the steps drive.
pub trait Target {
    /// Give input to the machine.
    fn give(&mut self, i: Input);
    /// A number that changes each time the screen, or the rectangle `rect` of it, changes.
    fn look(&mut self, rect: Option<[u16; 4]>) -> Result<u64, String> {
        let _ = rect;
        Err("the machine has no screen".into())
    }
    /// Write the screen, or the rectangle `rect` (x0, y0, x1, y1) of it, to the PNG file `file`.
    fn snap(&mut self, file: &str, rect: Option<[u16; 4]>) -> Result<(), String> {
        let _ = (file, rect);
        Err("the machine has no screen".into())
    }
    /// Make each vCPU write its registers to the standard error.
    fn regs(&mut self) -> Result<(), String> {
        Err("the machine cannot write its registers".into())
    }
    /// Give the event `e` of the host to the window of the machine.
    fn post(&mut self, e: h9_gui::HostEvent) -> Result<(), String> {
        let _ = e;
        Err("the machine has no window".into())
    }
}

impl<F: FnMut(Input)> Target for F {
    fn give(&mut self, i: Input) {
        self(i);
    }
}

/// The rectangle of the numbers `x0 y0 x1 y1`, or None when it is empty or a number is too large.
fn rect(x0: u64, y0: u64, x1: u64, y1: u64) -> Option<[u16; 4]> {
    let r = [x0, y0, x1, y1].map(u16::try_from);
    let [Ok(x0), Ok(y0), Ok(x1), Ok(y1)] = r else {
        return None;
    };
    (x0 < x1 && y0 < y1).then_some([x0, y0, x1, y1])
}

/// The `quiet` step of the words after `quiet`.
fn quiet_step(arg: &str) -> Option<Step> {
    let nums: Vec<u64> = arg
        .split_whitespace()
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    Some(match nums[..] {
        [] => Step::Quiet(STILL, QUIET_LIMIT, None),
        [ms] => Step::Quiet(Duration::from_millis(ms), QUIET_LIMIT, None),
        [ms, secs] => Step::Quiet(Duration::from_millis(ms), Duration::from_secs(secs), None),
        [ms, secs, x0, y0, x1, y1] => Step::Quiet(
            Duration::from_millis(ms),
            Duration::from_secs(secs),
            Some(rect(x0, y0, x1, y1)?),
        ),
        _ => return None,
    })
}

/// The modifier flags of the words `shift`, `control`, `option` and `command`.
fn mods(words: &[&str]) -> Option<u64> {
    let mut f = 0;
    for w in words {
        f |= match *w {
            "shift" => h9_gui::SHIFT,
            "control" => h9_gui::CONTROL,
            "option" => h9_gui::OPTION,
            "command" => h9_gui::COMMAND,
            _ => return None,
        };
    }
    Some(f)
}

/// The `post` step of the words after `post`.
fn post_step(arg: &str) -> Option<Step> {
    use h9_gui::HostEvent;
    let w: Vec<&str> = arg.split_whitespace().collect();
    let n = |i: usize| w.get(i).and_then(|v| v.parse::<u16>().ok());
    let e = match w[..] {
        ["key", _, dir, ref m @ ..] => HostEvent::Key {
            code: n(1)?,
            down: match dir {
                "down" => true,
                "up" => false,
                _ => return None,
            },
            flags: mods(m)?,
        },
        ["flags", _, ref m @ ..] => HostEvent::Flags {
            code: n(1)?,
            flags: mods(m)?,
        },
        [dir @ ("down" | "up"), _, _, b] => HostEvent::Button {
            x: n(1)?,
            y: n(2)?,
            button: b.parse().ok().filter(|&b: &u8| b < 32)?,
            down: dir == "down",
        },
        ["move", _, _] => HostEvent::Move { x: n(1)?, y: n(2)? },
        ["wheel", _, _, dir @ ("up" | "down")] => HostEvent::Wheel {
            x: n(1)?,
            y: n(2)?,
            up: dir == "up",
        },
        _ => return None,
    };
    Some(Step::Post(e))
}

/// The `snap` step of the words after `snap`.
fn snap_step(arg: &str) -> Option<Step> {
    let mut words = arg.split_whitespace();
    let file = words.next()?.to_string();
    let nums: Vec<u64> = words.map(str::parse).collect::<Result<_, _>>().ok()?;
    match nums[..] {
        [] => Some(Step::Snap(file, None)),
        [x0, y0, x1, y1] => Some(Step::Snap(file, Some(rect(x0, y0, x1, y1)?))),
        _ => None,
    }
}

/// Wait until the screen, or the rectangle `rect` of it, did not change for `still`, for no more
/// than `limit`.
fn quiet(
    t: &mut impl Target,
    still: Duration,
    limit: Duration,
    rect: Option<[u16; 4]>,
) -> Result<(), String> {
    let start = Instant::now();
    let mut g = t.look(rect)?;
    let mut since = start;
    loop {
        std::thread::sleep(Duration::from_millis(10).min(still));
        let now = Instant::now();
        let n = t.look(rect)?;
        if n != g {
            g = n;
            since = now;
        } else if now - since >= still {
            return Ok(());
        }
        if now - start >= limit {
            return Err(format!(
                "the screen did not stay the same for {} ms in {} s",
                still.as_millis(),
                limit.as_secs()
            ));
        }
    }
}

fn text(s: &str) -> Result<Vec<u8>, String> {
    let Some(q) = s.strip_prefix('"') else {
        return Ok(s.as_bytes().to_vec());
    };
    let mut out = Vec::new();
    let mut it = q.chars();
    loop {
        match it.next() {
            None => return Err("the string has no closing quote".into()),
            Some('"') => break,
            Some('\\') => match it.next() {
                Some('n') => out.push(b'\n'),
                Some('r') => out.push(b'\r'),
                Some('t') => out.push(b'\t'),
                Some('\\') => out.push(b'\\'),
                Some('"') => out.push(b'"'),
                Some('x') => {
                    let h: String = it.by_ref().take(2).collect();
                    let b = u8::from_str_radix(&h, 16)
                        .ok()
                        .filter(|_| h.len() == 2)
                        .ok_or(format!("\\x{h} is not a byte in hexadecimal"))?;
                    out.push(b);
                }
                c => return Err(format!("the escape \\{} is not known", c.unwrap_or(' '))),
            },
            Some(c) => out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes()),
        }
    }
    if !it.as_str().trim().is_empty() {
        return Err("text follows the closing quote".into());
    }
    Ok(out)
}

/// Read a script. Each step has the number of its line.
pub fn parse(src: &str) -> Result<Vec<(usize, Step)>, String> {
    let mut steps = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let n = i + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (verb, arg) = line.split_once(' ').unwrap_or((line, ""));
        let arg = arg.trim_start();
        let step = match verb {
            "wait" | "send" | "absent" => {
                let t = text(arg).map_err(|e| format!("line {n}: {e}"))?;
                if t.is_empty() {
                    return Err(format!("line {n}: {verb} needs a text"));
                }
                match verb {
                    "wait" => Step::Wait(t),
                    "absent" => Step::Absent(t),
                    _ => Step::Send(t),
                }
            }
            "timeout" | "pause" => {
                let d = Duration::from_secs(
                    arg.parse()
                        .map_err(|_| format!("line {n}: {verb} needs a number of seconds"))?,
                );
                if verb == "timeout" {
                    Step::Timeout(d)
                } else {
                    Step::Pause(d)
                }
            }
            "mouse" | "click" => {
                let bad = || format!("line {n}: give {verb} X Y and a button number");
                let nums: Vec<u32> = arg
                    .split_whitespace()
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .map_err(|_| bad())?;
                let x = nums.first().and_then(|&v| u16::try_from(v).ok());
                let y = nums.get(1).and_then(|&v| u16::try_from(v).ok());
                let button = nums.get(2).copied().unwrap_or(u32::from(verb == "click"));
                let (Some(x), Some(y), 2..=3, 0..=31) = (x, y, nums.len(), button) else {
                    return Err(bad());
                };
                let b = button as u8;
                if verb == "click" {
                    steps.push((n, Step::Mouse(x, y, 0)));
                    steps.push((n, Step::Mouse(x, y, b)));
                    Step::Mouse(x, y, 0)
                } else {
                    Step::Mouse(x, y, b)
                }
            }
            "quiet" => quiet_step(arg).ok_or_else(|| {
                format!(
                    "line {n}: give quiet, quiet MS, quiet MS SECS or quiet MS SECS X0 Y0 X1 Y1"
                )
            })?,
            "snap" => snap_step(arg)
                .ok_or_else(|| format!("line {n}: give snap FILE or snap FILE X0 Y0 X1 Y1"))?,
            "regs" if arg.is_empty() => Step::Regs,
            "post" => post_step(arg).ok_or_else(|| {
                format!(
                    "line {n}: give post key CODE down|up [MODS], post flags CODE [MODS], post down|up X Y BUTTON, post move X Y or post wheel X Y up|down"
                )
            })?,
            "quit" if arg.is_empty() => Step::Quit,
            _ => return Err(format!("line {n}: the step {line:?} is not known")),
        };
        steps.push((n, step));
    }
    Ok(steps)
}

/// The output of the console, as the script sees it. The console device writes to a `Tee`, and
/// the script waits on the transcript.
#[derive(Clone, Default)]
pub struct Transcript(Arc<(Mutex<Vec<u8>>, Condvar)>);

impl Transcript {
    fn add(&self, b: &[u8]) {
        self.0.0.lock().unwrap().extend_from_slice(b);
        self.0.1.notify_all();
    }

    /// Wait until `needle` is in the transcript after `from`, and give the position after it.
    /// None when `deadline` comes first.
    fn find(&self, needle: &[u8], from: usize, deadline: Instant) -> Option<usize> {
        let mut t = self.0.0.lock().unwrap();
        loop {
            if let Some(at) = t
                .get(from..)
                .and_then(|s| s.windows(needle.len()).position(|w| w == needle))
            {
                return Some(from + at + needle.len());
            }
            let now = Instant::now();
            if now >= deadline {
                return None;
            }
            t = self.0.1.wait_timeout(t, deadline - now).unwrap().0;
        }
    }

    /// Fail when `needle` is in the transcript after `from`.
    fn absent(&self, needle: &[u8], from: usize) -> Result<(), String> {
        match self.find(needle, from, Instant::now()) {
            Some(_) => Err(format!(
                "the console wrote {:?}",
                String::from_utf8_lossy(needle)
            )),
            None => Ok(()),
        }
    }
}

/// A writer that gives each byte to `out` and to the transcript.
pub struct Tee<W> {
    /// The writer of the console, as without a script.
    pub out: W,
    /// The copy for the script.
    pub log: Transcript,
}

impl<W: std::io::Write> std::io::Write for Tee<W> {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let n = self.out.write(b)?;
        self.log.add(&b[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.out.flush()
    }
}

/// The console output for the `wait` steps of the control socket: the transcript, the position
/// after the last match, and the time that each `wait` can take.
pub struct Watch {
    log: Transcript,
    seen: usize,
    timeout: Duration,
}

impl Watch {
    /// A watch of `log` from its start, with the default timeout.
    #[must_use]
    pub fn new(log: Transcript) -> Watch {
        Watch {
            log,
            seen: 0,
            timeout: TIMEOUT,
        }
    }
}

/// Do one line that a host program wrote on the socket of `--control`, and give the reply: `ok`,
/// or `error: ` and the reason. The line has the steps `send`, `mouse`, `click`, `pause`, `quiet`,
/// `snap`, `regs` and `post`, and `wait`, `absent` and `timeout` when `watch` has the console output. The monitor does all
/// the steps of the line before it replies, so a program that waits for the reply knows that the
/// input is in the machine.
pub fn control(line: &str, t: &mut impl Target, mut watch: Option<&mut Watch>) -> String {
    let steps = match parse(line) {
        Ok(s) => s,
        Err(e) => return format!("error: {}", e.trim_start_matches("line 1: ")),
    };
    let console = watch.is_some();
    if steps.iter().any(|(_, s)| {
        matches!(s, Step::Quit)
            || !console && matches!(s, Step::Wait(_) | Step::Absent(_) | Step::Timeout(_))
    }) {
        return format!(
            "error: {:?} is not a step of the control socket",
            line.trim()
        );
    }
    for (_, s) in &steps {
        let r = match s {
            Step::Send(b) => {
                t.give(Input::Keys(b));
                Ok(())
            }
            Step::Mouse(x, y, b) => {
                t.give(Input::Mouse(*x, *y, *b));
                Ok(())
            }
            Step::Pause(d) => {
                std::thread::sleep(*d);
                Ok(())
            }
            Step::Quiet(still, limit, r) => quiet(t, *still, *limit, *r),
            Step::Snap(f, r) => t.snap(f, *r),
            Step::Regs => t.regs(),
            Step::Post(e) => t.post(*e),
            Step::Wait(text) => match watch.as_deref_mut() {
                Some(w) => match w.log.find(text, w.seen, Instant::now() + w.timeout) {
                    Some(at) => {
                        w.seen = at;
                        Ok(())
                    }
                    None => Err(format!(
                        "the console did not write {:?} before the timeout of {} s",
                        String::from_utf8_lossy(text),
                        w.timeout.as_secs()
                    )),
                },
                None => Ok(()),
            },
            Step::Absent(text) => match watch.as_deref() {
                Some(w) => w.log.absent(text, w.seen),
                None => Ok(()),
            },
            Step::Timeout(d) => {
                if let Some(w) = watch.as_deref_mut() {
                    w.timeout = *d;
                }
                Ok(())
            }
            Step::Quit => Ok(()),
        };
        if let Err(e) = r {
            return format!("error: {e}");
        }
    }
    "ok".into()
}

/// The input that a step gives to the machine.
#[derive(Debug, PartialEq, Eq)]
pub enum Input<'a> {
    /// Bytes for the console.
    Keys(&'a [u8]),
    /// A mouse event: the point and the buttons.
    Mouse(u16, u16, u8),
}

/// Run the steps against the console. `send` gives the input of the steps to the machine. The result is Ok at `quit` or
/// at the end of the steps, and the reason of the failure otherwise.
pub fn run(steps: &[(usize, Step)], log: &Transcript, t: &mut impl Target) -> Result<(), String> {
    let mut timeout = TIMEOUT;
    let mut seen = 0;
    for (n, step) in steps {
        match step {
            Step::Wait(t) => {
                seen = log.find(t, seen, Instant::now() + timeout).ok_or_else(|| {
                    format!(
                        "line {n}: the console did not write {:?} before the timeout of {} s",
                        String::from_utf8_lossy(t),
                        timeout.as_secs()
                    )
                })?;
            }
            Step::Absent(t) => log.absent(t, seen).map_err(|e| format!("line {n}: {e}"))?,
            Step::Send(b) => t.give(Input::Keys(b)),
            Step::Mouse(x, y, b) => t.give(Input::Mouse(*x, *y, *b)),
            Step::Timeout(d) => timeout = *d,
            Step::Pause(d) => std::thread::sleep(*d),
            Step::Quiet(still, limit, r) => {
                quiet(t, *still, *limit, *r).map_err(|e| format!("line {n}: {e}"))?;
            }
            Step::Snap(f, r) => t.snap(f, *r).map_err(|e| format!("line {n}: {f}: {e}"))?,
            Step::Regs => t.regs().map_err(|e| format!("line {n}: {e}"))?,
            Step::Post(e) => t.post(*e).map_err(|e| format!("line {n}: {e}"))?,
            Step::Quit => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Input, Step, Target, Tee, Transcript, control, parse, run};
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn the_post_steps_are_read() {
        use h9_gui::{CONTROL, HostEvent, SHIFT};
        let s = parse(
            "post key 0 down shift control\npost key 9 up\npost flags 56 shift\npost flags 56\npost down 10 20 2\npost up 10 20 0\npost move 1023 767\npost wheel 5 6 up\npost wheel 5 6 down\n",
        )
        .unwrap();
        let steps: Vec<Step> = s.into_iter().map(|(_, s)| s).collect();
        assert_eq!(
            steps,
            vec![
                Step::Post(HostEvent::Key {
                    code: 0,
                    down: true,
                    flags: SHIFT | CONTROL
                }),
                Step::Post(HostEvent::Key {
                    code: 9,
                    down: false,
                    flags: 0
                }),
                Step::Post(HostEvent::Flags {
                    code: 56,
                    flags: SHIFT
                }),
                Step::Post(HostEvent::Flags { code: 56, flags: 0 }),
                Step::Post(HostEvent::Button {
                    x: 10,
                    y: 20,
                    button: 2,
                    down: true
                }),
                Step::Post(HostEvent::Button {
                    x: 10,
                    y: 20,
                    button: 0,
                    down: false
                }),
                Step::Post(HostEvent::Move { x: 1023, y: 767 }),
                Step::Post(HostEvent::Wheel {
                    x: 5,
                    y: 6,
                    up: true
                }),
                Step::Post(HostEvent::Wheel {
                    x: 5,
                    y: 6,
                    up: false
                }),
            ]
        );
        for bad in [
            "post",
            "post key a down",
            "post key 0 sideways",
            "post key 0 down hyper",
            "post down 10 20",
            "post down 10 20 40",
            "post move 70000 1",
            "post wheel 1 2 left",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        let mut none = |_: Input| {};
        assert!(control("post move 1 2", &mut none, None).contains("no window"));
    }

    #[test]
    fn a_script_is_read() {
        let s = parse(
            "# boot\nwait init: starting /bin/rc\nsend \"echo hi\\n\"\ntimeout 5\nwait \"hi\\x0a\"\npause 2\nquit\n",
        )
        .unwrap();
        assert_eq!(
            s,
            vec![
                (2, Step::Wait(b"init: starting /bin/rc".to_vec())),
                (3, Step::Send(b"echo hi\n".to_vec())),
                (4, Step::Timeout(Duration::from_secs(5))),
                (5, Step::Wait(b"hi\n".to_vec())),
                (6, Step::Pause(Duration::from_secs(2))),
                (7, Step::Quit),
            ]
        );
        assert!(parse("send \"a").unwrap_err().contains("line 1"));
        assert!(parse("send \"a\\q\"").unwrap_err().contains("\\q"));
        assert!(parse("send \"\\xg1\"").is_err());
        assert!(parse("send \"a\" b").is_err());
        assert!(parse("wait").unwrap_err().contains("needs a text"));
        assert!(parse("timeout soon").is_err());
        assert!(parse("pause").is_err());
        assert_eq!(
            parse("mouse 10 20\nmouse 10 20 4\nclick 5 6\nclick 5 6 2").unwrap(),
            vec![
                (1, Step::Mouse(10, 20, 0)),
                (2, Step::Mouse(10, 20, 4)),
                (3, Step::Mouse(5, 6, 0)),
                (3, Step::Mouse(5, 6, 1)),
                (3, Step::Mouse(5, 6, 0)),
                (4, Step::Mouse(5, 6, 0)),
                (4, Step::Mouse(5, 6, 2)),
                (4, Step::Mouse(5, 6, 0)),
            ]
        );
        for bad in [
            "mouse",
            "mouse 1",
            "mouse 1 2 3 4",
            "mouse 1 2 32",
            "click -1 2",
            "click a b",
            "mouse 70000 1",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("window 1024x768").unwrap_err().contains("not known"));
        assert_eq!(
            parse("quiet\nquiet 200\nquiet 200 5\nsnap a.png\nsnap b.png 1 2 30 40").unwrap(),
            vec![
                (
                    1,
                    Step::Quiet(Duration::from_millis(500), Duration::from_secs(30), None)
                ),
                (
                    2,
                    Step::Quiet(Duration::from_millis(200), Duration::from_secs(30), None)
                ),
                (
                    3,
                    Step::Quiet(Duration::from_millis(200), Duration::from_secs(5), None)
                ),
                (4, Step::Snap("a.png".into(), None)),
                (5, Step::Snap("b.png".into(), Some([1, 2, 30, 40]))),
            ]
        );
        assert_eq!(
            parse("quiet 100 2 0 0 10 10").unwrap(),
            vec![(
                1,
                Step::Quiet(
                    Duration::from_millis(100),
                    Duration::from_secs(2),
                    Some([0, 0, 10, 10])
                )
            )]
        );
        for bad in [
            "quiet a",
            "quiet 1 2 3",
            "quiet 1 2 3 4 5 6 7",
            "quiet 1 2 5 5 5 9",
            "quiet 1 2 0 0 70000 1",
            "snap",
            "snap a 1 2 3",
            "snap a 5 5 5 9",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
        assert!(parse("quit now").is_err());
    }

    #[test]
    fn the_script_follows_the_console() {
        let log = Transcript::default();
        let mut tee = Tee {
            out: Vec::new(),
            log: log.clone(),
        };
        let steps =
            parse("wait term% \nsend ls\\n\ntimeout 1\nwait \"term% \"\nquit\nwait never").unwrap();
        let t = std::thread::spawn({
            let log = log.clone();
            move || {
                let mut sent = Vec::new();
                let r = run(&steps, &log, &mut |i: Input<'_>| match i {
                    Input::Keys(b) => sent.extend_from_slice(b),
                    Input::Mouse(..) => panic!("no mouse step"),
                });
                (r, sent)
            }
        });
        tee.write_all(b"boot\nterm% ").unwrap();
        std::thread::sleep(Duration::from_millis(50));
        tee.write_all(b"ls\nbin\nterm% ").unwrap();
        let (r, sent) = t.join().unwrap();
        assert_eq!(r, Ok(()));
        assert_eq!(sent, b"ls\\n");
        assert_eq!(tee.out, b"boot\nterm% ls\nbin\nterm% ");

        let steps = parse("timeout 0\nwait \"term% \"\nwait \"term% \"\nwait \"term% \"").unwrap();
        let r = run(&steps, &log, &mut |_: Input| {});
        assert!(
            r.unwrap_err().contains("line 4"),
            "each match is after the last one"
        );
    }

    /// A screen that changes at each of the first `busy` reads of the generation.
    struct Screen {
        busy: u64,
        reads: u64,
        snaps: Vec<(String, Option<[u16; 4]>)>,
    }

    impl Target for Screen {
        fn give(&mut self, _: Input) {}

        fn look(&mut self, _: Option<[u16; 4]>) -> Result<u64, String> {
            self.reads += 1;
            Ok(self.reads.min(self.busy))
        }

        fn snap(&mut self, file: &str, rect: Option<[u16; 4]>) -> Result<(), String> {
            self.snaps.push((file.into(), rect));
            Ok(())
        }
    }

    #[test]
    fn quiet_waits_for_the_screen_and_snap_writes_it() {
        let mut s = Screen {
            busy: 5,
            reads: 0,
            snaps: Vec::new(),
        };
        assert_eq!(control("quiet 30 5", &mut s, None), "ok");
        assert!(s.reads > 5, "quiet reads until the screen stops");
        assert_eq!(control("snap a.png 0 0 8 8", &mut s, None), "ok");
        assert_eq!(s.snaps, vec![("a.png".to_string(), Some([0, 0, 8, 8]))]);

        let mut busy = Screen {
            busy: u64::MAX,
            reads: 0,
            snaps: Vec::new(),
        };
        let r = control("quiet 50 0", &mut busy, None);
        assert!(r.contains("did not stay the same"), "{r}");

        let mut none = |_: Input| {};
        assert!(control("quiet", &mut none, None).contains("no screen"));
        assert!(control("snap a.png", &mut none, None).contains("no screen"));
    }

    #[test]
    fn a_control_line_gives_input_and_a_reply() {
        let mut got = Vec::new();
        let mut give = |i: Input| {
            got.push(match i {
                Input::Keys(b) => format!("keys {}", String::from_utf8_lossy(b)),
                Input::Mouse(x, y, b) => format!("mouse {x} {y} {b}"),
            });
        };
        assert_eq!(control("click 300 300", &mut give, None), "ok");
        assert_eq!(control("send \"ls\\n\"", &mut give, None), "ok");
        assert_eq!(control("pause 0", &mut give, None), "ok");
        assert!(control("wait term%", &mut give, None).starts_with("error: "));
        assert!(control("quit", &mut give, None).starts_with("error: "));
        let log = Transcript::default();
        let mut w = super::Watch::new(log.clone());
        log.add(b"R9-GUI-EXIT 1\nR9-GUI-EXIT \n");
        assert_eq!(control("wait R9-GUI-EXIT", &mut give, Some(&mut w)), "ok");
        assert_eq!(control("wait R9-GUI-EXIT", &mut give, Some(&mut w)), "ok");
        assert_eq!(control("absent R9-GUI-EXIT", &mut give, Some(&mut w)), "ok");
        log.add(b"R9-GUI-EXIT 2\n");
        let r = control("absent R9-GUI-EXIT", &mut give, Some(&mut w));
        assert!(r.contains("the console wrote"), "{r}");
        assert!(control("absent X", &mut give, None).starts_with("error: "));
        assert_eq!(control("wait R9-GUI-EXIT", &mut give, Some(&mut w)), "ok");
        assert_eq!(control("timeout 0", &mut give, Some(&mut w)), "ok");
        let r = control("wait R9-GUI-EXIT", &mut give, Some(&mut w));
        assert!(r.contains("before the timeout of 0 s"), "{r}");
        assert_eq!(
            control("mouse 1", &mut give, None),
            "error: give mouse X Y and a button number"
        );
        assert_eq!(
            got,
            [
                "mouse 300 300 0",
                "mouse 300 300 1",
                "mouse 300 300 0",
                "keys ls\n"
            ]
        );
    }
}
