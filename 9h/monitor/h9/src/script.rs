//! The session scripts of `9h run --script FILE` (plan 10.8 and 12.6). A script drives the
//! console of a session with no person at it, as `r9 9h boot` and `r9 9h smoke` do. Each line is
//! one step:
//! - `wait TEXT` waits until the console writes TEXT after the last match;
//! - `send TEXT` types TEXT on the console;
//! - `timeout SECS` sets the time that each later `wait` can take, 60 seconds at the start;
//! - `pause SECS` waits SECS seconds, such as for a program that does not write to the console;
//! - `quit` ends the session, and the script passes.
//!
//! TEXT is the rest of the line with the spaces at its two ends removed, or a string in double quotes with the escapes `\n`, `\r`, `\t`,
//! `\\`, `\"` and `\xHH`. A line that starts with `#` is a comment. The script fails at the first
//! `wait` that times out, and when the session ends before the script does. The steps of the GUI
//! (`window`, `mouse`, `keys`, `snap`, `quiet`) come with the screen.

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

/// Run the steps against the console. `send` types bytes on it. The result is Ok at `quit` or
/// at the end of the steps, and the reason of the failure otherwise.
pub fn run(
    steps: &[(usize, Step)],
    log: &Transcript,
    mut send: impl FnMut(&[u8]),
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
            Step::Send(t) => send(t),
            Step::Timeout(d) => timeout = *d,
            Step::Pause(d) => std::thread::sleep(*d),
            Step::Quit => break,
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Step, Tee, Transcript, parse, run};
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
                let r = run(&steps, &log, |b| sent.extend_from_slice(b));
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
}
