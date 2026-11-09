//! The serial console of a 9front guest (plan 12.2).
//!
//! The release images are not changed, so nothing in the guest prints markers on its own. `r9
//! boot` answers the boot prompts and types each command at the shell prompt, followed by
//! `echo R9-STATUS $status`. rc prints the status of the command on that line, which is empty
//! when the command succeeded.
//!
//! What the console shows, in order:
//!
//! | text | answer |
//! |---|---|
//! | `bootargs is (tcp, tls, il, local!device)[…]` | the default, or `--bootargs` |
//! | `user[glenda]: ` | the default |
//! | `term% ` or `cpu% ` | the next command |
//! | `R9-STATUS <status>` | the result of the command before it |
//! | `done halting` | the end, after `fshalt` |

/// Splits console bytes into lines. A prompt such as `user[glenda]: ` has no newline after it,
/// so the part after the last newline is kept and can be looked at.
#[derive(Debug, Default)]
pub struct Lines {
    partial: Vec<u8>,
}

impl Lines {
    /// Take more bytes, and return the lines they end, without `\r` and `\n`.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                out.push(text(&self.partial));
                self.partial.clear();
            } else {
                self.partial.push(b);
            }
        }
        out
    }

    /// The text after the last newline.
    #[must_use]
    pub fn partial(&self) -> String {
        text(&self.partial)
    }

    /// Forget the text after the last newline, once a prompt in it has been answered.
    pub fn answered(&mut self) {
        self.partial.clear();
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\r')
        .replace('\r', "")
}

/// The marker line after each command.
pub const STATUS: &str = "R9-STATUS";

/// One command and what came of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// The command as typed.
    pub command: String,
    /// The status rc gave, which is empty for success, or `None` when the guest never answered.
    pub status: Option<String>,
    /// The lines the command printed.
    pub output: Vec<String>,
}

/// Where the guest is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// The kernel and the boot scripts are running.
    Booting,
    /// A command is running.
    Running,
    /// `fshalt` was typed.
    Halting,
    /// The guest said `done halting`.
    Halted,
    /// The kernel panicked.
    Panicked,
}

/// What to type next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Nothing yet.
    Wait,
    /// Type this text.
    Type(String),
}

/// The console automaton: it reads lines and prompts and says what to type.
#[derive(Debug)]
pub struct Session {
    /// The answer to the `bootargs` prompt, or empty for the default.
    pub bootargs: String,
    /// The commands still to run.
    pending: std::collections::VecDeque<String>,
    /// The commands that ran.
    pub steps: Vec<Step>,
    /// Where the guest is.
    pub phase: Phase,
    /// The panic line, when there was one.
    pub panic: Option<String>,
    /// Whether the shell prompt was seen.
    pub ready: bool,
}

impl Session {
    /// A session that runs `commands` and then halts the guest.
    #[must_use]
    pub fn new(bootargs: &str, commands: Vec<String>) -> Self {
        Self {
            bootargs: bootargs.to_string(),
            pending: commands.into(),
            steps: Vec::new(),
            phase: Phase::Booting,
            panic: None,
            ready: false,
        }
    }

    /// Look at one whole line.
    pub fn line(&mut self, line: &str) {
        let panic =
            line.starts_with("panic:") || line.starts_with("cpu") && line.contains(" panic: ");
        if panic && self.panic.is_none() {
            self.panic = Some(line.to_string());
            self.phase = Phase::Panicked;
            return;
        }
        if line.contains("done halting") {
            self.phase = Phase::Halted;
            return;
        }
        if self.phase != Phase::Running {
            return;
        }
        let Some(step) = self.steps.last_mut() else {
            return;
        };
        if is_echo(line, &step.command) {
            return;
        }
        // A command whose output has no newline at the end puts the marker in the middle of the
        // line, as in `cirnoR9-STATUS `.
        if let Some(at) = line.rfind(STATUS) {
            let (before, rest) = line.split_at(at);
            if !before.is_empty() {
                step.output.push(before.to_string());
            }
            step.status = Some(rest[STATUS.len()..].trim().to_string());
        } else {
            step.output.push(line.to_string());
        }
    }

    /// Look at the text after the last line, and say what to type. The caller forgets that
    /// text with [`Lines::answered`] when this returns [`Action::Type`].
    pub fn prompt(&mut self, partial: &str) -> Action {
        let partial = partial.trim_end();
        if self.phase == Phase::Booting && partial.contains("bootargs is") && partial.ends_with(']')
        {
            return Action::Type(format!("{}\n", self.bootargs));
        }
        if self.phase == Phase::Booting && partial.starts_with("user[") && partial.ends_with(':') {
            return Action::Type("\n".into());
        }
        if !is_shell_prompt(partial) {
            return Action::Wait;
        }
        self.ready = true;
        let done = self.steps.last().is_none_or(|s| s.status.is_some());
        if !done || matches!(self.phase, Phase::Halting | Phase::Halted | Phase::Panicked) {
            return Action::Wait;
        }
        if let Some(command) = self.pending.pop_front() {
            self.phase = Phase::Running;
            self.steps.push(Step {
                command: command.clone(),
                status: None,
                output: Vec::new(),
            });
            Action::Type(format!("{command}; echo {STATUS} $status\n"))
        } else {
            self.phase = Phase::Halting;
            Action::Type("fshalt\n".into())
        }
    }

    /// Whether every command ran and succeeded.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.pending.is_empty()
            && self.panic.is_none()
            && self
                .steps
                .iter()
                .all(|s| s.status.as_deref().is_some_and(str::is_empty))
    }
}

/// Whether text is rc's prompt with nothing typed after it.
fn is_shell_prompt(text: &str) -> bool {
    matches!(text, "term%" | "cpu%" | "%")
}

/// Whether a line is the echo of the command as typed.
fn is_echo(line: &str, command: &str) -> bool {
    line.contains(&format!("{command}; echo {STATUS}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(s: &mut Session, lines: &mut Lines, text: &str) -> Vec<String> {
        let mut typed = Vec::new();
        for l in lines.push(text.as_bytes()) {
            s.line(&l);
        }
        if let Action::Type(t) = s.prompt(&lines.partial()) {
            lines.answered();
            typed.push(t);
        }
        typed
    }

    #[test]
    fn a_boot_runs_the_commands_and_halts() {
        let mut s = Session::new("", vec!["cat /dev/sysname".into(), "6c -h".into()]);
        let mut l = Lines::default();
        assert!(feed(&mut s, &mut l, "Plan 9\r\n126 holes free\r\n").is_empty());
        let t = feed(
            &mut s,
            &mut l,
            "/dev/sdF0/fs\r\nbootargs is (tcp, tls, il, local!device)[local!/dev/sdF0/fs] ",
        );
        assert_eq!(t, vec!["\n"]);
        assert_eq!(feed(&mut s, &mut l, "user[glenda]: "), vec!["\n"]);
        let t = feed(&mut s, &mut l, "rio: can't open display\r\nterm% ");
        assert_eq!(t, vec!["cat /dev/sysname; echo R9-STATUS $status\n"]);
        let t = feed(
            &mut s,
            &mut l,
            "term% cat /dev/sysname; echo R9-STATUS $status\r\ncirnoR9-STATUS \r\nterm% ",
        );
        assert_eq!(t, vec!["6c -h; echo R9-STATUS $status\n"]);
        let t = feed(
            &mut s,
            &mut l,
            "6c -h; echo R9-STATUS $status\r\nusage: 6c\r\nR9-STATUS 6c 1234: usage\r\nterm% ",
        );
        assert_eq!(t, vec!["fshalt\n"]);
        feed(&mut s, &mut l, "syncing...done halting\r\n");
        assert_eq!(s.phase, Phase::Halted);
        assert_eq!(s.steps[0].output, vec!["cirno"]);
        assert_eq!(s.steps[1].status.as_deref(), Some("6c 1234: usage"));
        assert!(!s.passed());
    }

    #[test]
    fn a_panic_stops_the_session() {
        let mut s = Session::new("", vec!["ls".into()]);
        let mut l = Lines::default();
        feed(&mut s, &mut l, "panic: fault in kernel\r\n");
        assert_eq!(s.phase, Phase::Panicked);
        assert!(feed(&mut s, &mut l, "term% ").is_empty());
        assert!(!s.passed());
    }

    #[test]
    fn a_prompt_in_the_middle_of_a_command_waits() {
        let mut s = Session::new("local!/dev/sdC0/fs", vec!["sleep 100".into()]);
        let mut l = Lines::default();
        let t = feed(
            &mut s,
            &mut l,
            "bootargs is (tcp, tls, il, local!device)[] ",
        );
        assert_eq!(t, vec!["local!/dev/sdC0/fs\n"]);
        feed(&mut s, &mut l, "term% ");
        assert!(
            feed(
                &mut s,
                &mut l,
                "sleep 100; echo R9-STATUS $status\r\nterm% "
            )
            .is_empty()
        );
    }
}
