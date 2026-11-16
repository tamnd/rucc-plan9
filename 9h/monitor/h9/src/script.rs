//! The session scripts of `9h run --script FILE` (plan 10.8 and 12.6). A script drives the
//! console of a session with no person at it, as `r9 9h boot` and `r9 9h smoke` do. Each line is
//! one step:
//! - `wait TEXT` waits until the console writes TEXT after the last match;
//! - `send TEXT` types TEXT on the console;
//! - `timeout SECS` sets the time that each later `wait` can take, 60 seconds at the start;
//! - `pause SECS` waits SECS seconds, such as for a program that does not write to the console;
//! - `mouse X Y [BUTTONS]` moves the mouse to the pixel (X, Y) of the screen with BUTTONS down, 0
//!   by default: 1 is the left button, 2 the middle, 4 the right, and 8 and 16 the wheel;
//! - `click X Y [BUTTON]` moves the mouse to (X, Y), and pushes and releases BUTTON, 1 by default;
//! - `quit` ends the session, and the script passes.
//!
//! TEXT is the rest of the line with the spaces at its two ends removed, or a string in double quotes with the escapes `\n`, `\r`, `\t`,
//! `\\`, `\"` and `\xHH`. A line that starts with `#` is a comment. The script fails at the first
//! `wait` that times out, and when the session ends before the script does. The other steps of the
//! GUI (`window`, `keys`, `snap`, `quiet`) come later.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// One step of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Wait for these bytes on the console.
    Wait(Vec<u8>),
    /// Type these bytes on the console.
    Send(Vec<u8>),
    /// The time that each later `wait` can take.
    Timeout(Duration),
    /// Wait for this time.
    Pause(Duration),
    /// Move the mouse to (x, y) with these buttons down.
    Mouse(u16, u16, u8),
    /// End the session.
    Quit,
}

/// The default time of a `wait`.
pub const TIMEOUT: Duration = Duration::from_secs(60);

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
            "wait" | "send" => {
                let t = text(arg).map_err(|e| format!("line {n}: {e}"))?;
                if t.is_empty() {
                    return Err(format!("line {n}: {verb} needs a text"));
                }
                if verb == "wait" {
                    Step::Wait(t)
                } else {
                    Step::Send(t)
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

/// Do one line that a host program wrote on the socket of `--control`, and give the reply: `ok`,
/// or `error: ` and the reason. The line has the steps `send`, `mouse`, `click` and `pause` only,
/// because the console output does not go to the socket. The monitor does all the steps of the
/// line before it replies, so a program that waits for the reply knows that the input is in the
/// machine.
pub fn control(line: &str, mut send: impl FnMut(Input)) -> String {
    let steps = match parse(line) {
        Ok(s) => s,
        Err(e) => return format!("error: {}", e.trim_start_matches("line 1: ")),
    };
    if steps
        .iter()
        .any(|(_, s)| !matches!(s, Step::Send(_) | Step::Mouse(..) | Step::Pause(_)))
    {
        return format!(
            "error: {:?} is not a step of the control socket",
            line.trim()
        );
    }
    for (_, s) in &steps {
        match s {
            Step::Send(t) => send(Input::Keys(t)),
            Step::Mouse(x, y, b) => send(Input::Mouse(*x, *y, *b)),
            Step::Pause(d) => std::thread::sleep(*d),
            _ => {}
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
pub fn run(
    steps: &[(usize, Step)],
    log: &Transcript,
    mut send: impl FnMut(Input),
) -> Result<(), String> {
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
            Step::Send(t) => send(Input::Keys(t)),
            Step::Mouse(x, y, b) => send(Input::Mouse(*x, *y, *b)),
            Step::Timeout(d) => timeout = *d,
            Step::Pause(d) => std::thread::sleep(*d),
            Step::Quit => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Input, Step, Tee, Transcript, control, parse, run};
    use std::io::Write;
    use std::time::Duration;

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
        assert!(parse("snap open").unwrap_err().contains("not known"));
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
                let r = run(&steps, &log, |i| match i {
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
        let r = run(&steps, &log, |_| {});
        assert!(
            r.unwrap_err().contains("line 4"),
            "each match is after the last one"
        );
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
        assert_eq!(control("click 300 300", &mut give), "ok");
        assert_eq!(control("send \"ls\\n\"", &mut give), "ok");
        assert_eq!(control("pause 0", &mut give), "ok");
        assert!(control("wait term%", &mut give).starts_with("error: "));
        assert!(control("quit", &mut give).starts_with("error: "));
        assert_eq!(
            control("mouse 1", &mut give),
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
