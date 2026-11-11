//! `r9 build --side G`: build a tree in a guest and collect what came of it (plan 05.3).
//!
//! The guest is a release image on a fresh overlay. The `guest` directory of the repository goes
//! in on a tar disk. The guest builds `r9-cc` with its own compiler, then runs `r9-build`, which
//! records every call of the compiler slot. After the build, the guest lists the files under the
//! tree and the install directories with their times, hashes each file that the build wrote,
//! and archives the generated C sources and headers. All of it comes back on the collect disk.
//!
//! On the host, the collected files are in `DIR/collected`, the generated sources are unpacked
//! into `DIR/generated`, and `DIR/build.toml` gives the summary.
//!
//! Side R puts rucc in the compiler slot (plan 05.4). `r9 build` starts `r9 ccd` on a free port of
//! the loopback address, and boots the guest with a network card. `r9-cc` sends each call for the
//! machine of the guest to the host. With `cc:auto` in `R9_BRINGUP` (the default), a call that
//! rucc fails runs the real compiler, and its record has the origin `ref-c`. What rucc wrote to
//! standard error for such calls is in `rucc.err`, and the line of each call is in `ccd.log`.

use crate::boot::{self, Request};
use crate::census;
use serde::Serialize;
use std::fmt::Write as _;
use std::path::Path;

/// The size of the collect disk.
const COLLECT_MIB: u64 = 128;

/// The files that the guest puts on the collect disk.
pub const COLLECTED: &[&str] = &[
    "t0",
    "t1",
    "status",
    "mk.log",
    "compile.jsonl",
    "walk",
    "changed",
    "changed.sha256",
    "members.sha256",
    "gen.tar",
    "rucc.err",
];

/// The compiler slot of a build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Slot {
    /// kencc: `r9-cc` records each call and runs the real compiler.
    G,
    /// rucc: `r9-cc` sends each call to `r9 ccd` on `port` of the host, with `bringup` in
    /// `$r9bringup`.
    R {
        /// The port of `r9 ccd`.
        port: u16,
        /// The bring-up words, such as `cc:auto`.
        bringup: String,
    },
}

impl Slot {
    /// `G` or `R`.
    #[must_use]
    pub const fn letter(&self) -> &'static str {
        match self {
            Self::G => "G",
            Self::R { .. } => "R",
        }
    }
}

/// The commands that the guest runs to build `dir` with `mk install`. Each command is one line
/// at the shell prompt.
#[must_use]
pub fn commands(dir: &str, slot: &Slot) -> Vec<String> {
    let w = "/tmp/r9";
    // rucc.err is append-only, because each call of r9-cc opens it to write.
    let (net, env) = match slot {
        Slot::G => (String::new(), String::new()),
        Slot::R { port, bringup } => (
            " && ip/ipconfig".to_string(),
            format!("r9ccd='tcp!10.0.2.2!{port}' r9bringup='{bringup}' "),
        ),
    };
    vec![
        format!(
            "mkdir -p /tmp/r9in /usr/glenda/r9/bin {w} && touch {w}/rucc.err && chmod +a {w}/rucc.err && cd /tmp/r9in && tar xf $r9in{net}"
        ),
        "cp /tmp/r9in/r9-build /usr/glenda/r9/bin/ && cd /tmp/r9in/r9-cc && mk install".into(),
        // A loop in the background prints a progress line to the console every minute: the
        // time, the lines of the log, its last line and the memory in use. The console log on
        // the host keeps it when the guest stops before its file server writes the log out.
        format!(
            "date -n > {w}/t0; {{while(! test -f {w}/t1){{sleep 60; echo r9-progress `{{date -n}} `{{wc -l < {w}/mk.log}} `{{tail -1 {w}/mk.log}}; grep memory /dev/swap}}}} & r9p=$apid; {env}/usr/glenda/r9/bin/r9-build {dir} install < /dev/null > {w}/mk.log >[2=1]; s=$status; date -n > {w}/t1; echo kill > /proc/$r9p/note; echo $s > {w}/status; test -z $s"
        ),
        format!("tail -20 {w}/mk.log"),
        format!("walk -f -e msp {dir} /$objtype/bin /$objtype/lib > {w}/walk"),
        format!(
            "t0=`{{cat {w}/t0}}; awk -v 't='$t0 '$1 >= t+0 {{print $3}}' {w}/walk > {w}/changed"
        ),
        // t0 is always named, so that an empty list does not make sha1sum read its input.
        format!("sha1sum -2 256 {w}/t0 `{{cat {w}/changed}} > {w}/changed.sha256"),
        // An archive records the date of each member, so r9 tree-diff compares the members.
        format!(
            "{{for(a in `{{grep '\\.a.?$' {w}/changed}}){{rm -rf {w}/ar; mkdir {w}/ar; cd {w}/ar; ar x $a; sha1sum -2 256 * | awk -v 'a='$a '{{print $1 \"\\t\" a \"(\" $2 \")\"}}'}}}} > {w}/members.sha256; cd / && rm -rf {w}/ar"
        ),
        // yacc -D writes y.debug, which yaccpar includes.
        format!("tar cf {w}/gen.tar {w}/t0 `{{grep '\\.[chy]$|/y\\.debug$' {w}/changed}}"),
        format!("cd {w} && tar cf $r9collect {}", COLLECTED.join(" ")),
    ]
}

/// A test that the guest runs after the build (plan 12.8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Test {
    /// The rc command.
    pub command: String,
    /// The longest time of the test. After it, the guest kills the test.
    pub seconds: u64,
}

/// The files of a test on the collect disk.
pub const TEST_FILES: &[&str] = &["test.status", "test.log"];

/// The command that runs `test` in the guest, after the build.
///
/// The test runs in the background in a new note group, so that the watchdog can kill the test
/// and each process it started, and not the shell of the console. The watchdog writes `timeout`
/// to `test.status` and then kills the group. When the test exits first, its status goes to
/// `test.status`, and an empty status is a pass. After a timeout the watchdog is gone, and a
/// write to its note file fails. In the console rc, a failed redirection stops the rest of the line,
/// so the command writes the note only when the process is there.
#[must_use]
pub fn test_command(test: &Test) -> String {
    let w = "/tmp/r9";
    format!(
        "rm -f {w}/test.status; {{rfork s; {cmd}}} < /dev/null > {w}/test.log >[2=1] & r9t=$apid; {{sleep {n}; echo timeout > {w}/test.status; echo kill > /proc/$r9t/notepg}} & r9w=$apid; wait $r9t; s=$status; test -d /proc/$r9w && echo kill > /proc/$r9w/note; test -f {w}/test.status || echo $s > {w}/test.status; echo r9-test `{{cat {w}/test.status}}",
        cmd = test.command,
        n = test.seconds
    )
}

/// The summary in `build.toml`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Summary {
    /// The release that was booted.
    pub release: String,
    /// The `objtype`.
    pub objtype: String,
    /// `G` for kencc, `R` for rucc.
    pub side: String,
    /// The tree that was built.
    pub dir: String,
    /// The image that was booted.
    pub image: String,
    /// The accelerator of QEMU.
    pub accel: String,
    /// Whether `mk install` exited with no error.
    pub passed: bool,
    /// The exit status of `mk install`, empty for success.
    pub status: String,
    /// The seconds of `mk install` by the guest's clock.
    pub build_seconds: u64,
    /// The seconds of the whole boot by the host's clock.
    pub boot_seconds: f64,
    /// The calls of the compiler slot.
    pub calls: usize,
    /// The calls that failed.
    pub failed_calls: usize,
    /// The calls that rucc compiled.
    pub rucc_calls: usize,
    /// The calls for the machine of the guest that went to kencc.
    pub kencc_calls: usize,
    /// The files that the build wrote.
    pub changed_files: usize,
    /// The status of the test: empty for a pass, `timeout`, or the exit status. With no test,
    /// there is no value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test_status: Option<String>,
}

/// Read a collected file as text, or an empty string when the guest did not write it.
fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_default()
}

/// Run `r9 build`. `req` is a boot request for the release; its commands, disks and limit are
/// set here. Return the summary and the report to print.
pub fn run(
    release: &str,
    dir: &str,
    mut req: Request,
    repo_root: &Path,
    slot: &Slot,
    guest_test: Option<&Test>,
) -> Result<(Summary, String), String> {
    req.commands = commands(dir, slot);
    if let Some(t) = guest_test {
        // After the tail of the log, and before the walk and the collection.
        req.commands.insert(4, test_command(t));
        if let Some(last) = req.commands.last_mut() {
            for f in TEST_FILES {
                last.push(' ');
                last.push_str(f);
            }
        }
    }
    req.net |= matches!(slot, Slot::R { .. });
    req.input = Some(repo_root.join("guest"));
    req.collect_mib = Some(COLLECT_MIB);
    eprintln!("r9 build: {}", boot::qemu_command(&req).join(" "));
    let outcome = boot::run(&req)?;
    let collected = req.work.join("collected");
    let generated = req.work.join("generated");
    let gen_tar = collected.join("gen.tar");
    if gen_tar.is_file() {
        std::fs::create_dir_all(&generated).map_err(|e| format!("{}: {e}", generated.display()))?;
        crate::disk::unpack(&gen_tar, &generated)?;
    }
    let records_path = collected.join("compile.jsonl");
    let records = if records_path.is_file() {
        census::load(&records_path)?
    } else {
        Vec::new()
    };
    let c = census::take(&records);
    let t0: u64 = read(&collected, "t0").trim().parse().unwrap_or(0);
    let t1: u64 = read(&collected, "t1").trim().parse().unwrap_or(0);
    let status = read(&collected, "status").trim().to_string();
    let summary = Summary {
        release: release.to_string(),
        objtype: req.objtype.clone(),
        side: slot.letter().into(),
        dir: dir.to_string(),
        image: req.base.display().to_string(),
        accel: req.accel.clone(),
        passed: t1 > 0 && status.is_empty(),
        status,
        build_seconds: t1.saturating_sub(t0),
        boot_seconds: outcome.seconds,
        calls: c.calls,
        failed_calls: c.failed.len(),
        rucc_calls: count(&records, "rucc"),
        kencc_calls: count(&records, "ref-c"),
        changed_files: read(&collected, "changed").lines().count(),
        test_status: guest_test.map(|_| {
            let text = read(&collected, "test.status");
            if text.is_empty() {
                "no status".to_owned()
            } else {
                text.trim().to_owned()
            }
        }),
    };
    let text = toml::to_string(&summary).map_err(|e| e.to_string())?;
    let path = req.work.join("build.toml");
    std::fs::write(
        &path,
        format!("# Written by r9 build. Do not edit.\n{text}"),
    )
    .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut report = boot::summary(&outcome, &req.work);
    let side = match slot {
        Slot::G => census::Side::G,
        Slot::R { .. } => census::Side::R,
    };
    report.push_str(&c.report(side));
    if let Some(t) = &summary.test_status {
        let _ = writeln!(
            report,
            "test {}",
            if t.is_empty() { "passed" } else { t.as_str() }
        );
    }
    let _ = writeln!(
        report,
        "build {} in {} s, {} files written; summary in {}",
        if summary.passed { "passed" } else { "failed" },
        summary.build_seconds,
        summary.changed_files,
        path.display()
    );
    Ok((summary, report))
}

/// `r9 build` and `r9 mix` from the command line. Return whether the build passed, and the
/// test too when there is one, and the report.
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(bool, String), String> {
    // `r9 mix` is side R with a mix.
    let mix = crate::mix::from_args(args)?;
    let r = match (args.command.as_str(), args.get("side")) {
        ("mix", _) | (_, Some("R" | "r")) => true,
        (_, Some("G" | "g")) => false,
        _ => return Err("give --side G or --side R".into()),
    };
    let bisect = args.has("bisect");
    if args.command == "mix" && mix.is_none() && !bisect {
        return Err("give the mix with --file, --mix or --default".into());
    }
    let test = match args.get("test") {
        None if bisect => return Err("--bisect needs --test".into()),
        None => None,
        Some(command) => Some(Test {
            command: command.to_owned(),
            seconds: args
                .get("test-timeout")
                .map_or(Ok(600), str::parse)
                .map_err(|e| format!("--test-timeout: {e}"))?,
        }),
    };
    let mut req = boot::request_from(repo, args)?;
    let release = req
        .base
        .file_name()
        .and_then(|f| f.to_str())
        .and_then(|f| f.strip_prefix("9front-"))
        .and_then(|f| f.split('.').next())
        .unwrap_or_default()
        .to_string();
    // R and G builds start from the g-base image when there is one (plan 05.2).
    if args.get("image").is_none()
        && let Some(g_base) = crate::image::g_base(repo, &req.objtype)
    {
        req.base = g_base;
    }
    if args.get("out").is_none() {
        req.work = repo.root.join("work").join("build").join(format!(
            "{release}-{}-{}",
            req.objtype,
            match (&mix, r) {
                _ if bisect => "bisect",
                (Some(_), _) => "mix",
                (None, true) => "R",
                (None, false) => "G",
            }
        ));
    }
    let rucc: std::path::PathBuf = args.get("rucc").map_or_else(
        || std::env::var_os("R9_RUCC").map_or_else(|| "rucc".into(), Into::into),
        Into::into,
    );
    let one = Build {
        release,
        dir: args.get("dir").unwrap_or("/sys/src").to_owned(),
        repo_root: repo.root.clone(),
        rucc,
        test,
    };
    if bisect {
        return crate::mix::bisect_command(&one, &req, mix.is_some());
    }
    let (summary, report) = one.run(req, r.then_some(mix))?;
    let passed = summary.passed && summary.test_status.as_ref().is_none_or(String::is_empty);
    Ok((passed, report))
}

/// What each build of a command has in common.
#[derive(Debug, Clone)]
pub struct Build {
    /// The release of the image.
    pub release: String,
    /// The tree to build in the guest.
    pub dir: String,
    /// The root of the repository.
    pub repo_root: std::path::PathBuf,
    /// The rucc program for side R.
    pub rucc: std::path::PathBuf,
    /// The test to run after the build.
    pub test: Option<Test>,
}

impl Build {
    /// Run one build with the boot request `req`. With `side_r`, start r9 ccd with the mix, or
    /// with no mix when the value is `None`; with no `side_r`, build side G.
    pub fn run(
        &self,
        req: Request,
        side_r: Option<Option<crate::mix::Mix>>,
    ) -> Result<(Summary, String), String> {
        let slot = match side_r {
            None => Slot::G,
            Some(mix) => {
                let log = req.work.join("ccd.log");
                std::fs::create_dir_all(&req.work)
                    .map_err(|e| format!("{}: {e}", req.work.display()))?;
                let file =
                    std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
                let bringup = std::env::var("R9_BRINGUP").unwrap_or_else(|_| "cc:auto".into());
                if let Some(m) = &mix {
                    // r9-cc runs kencc for a call on side G only under cc:auto.
                    if !bringup.contains("cc:auto") {
                        return Err("a mix needs cc:auto in R9_BRINGUP".into());
                    }
                    let path = req.work.join("mix.txt");
                    std::fs::write(&path, m.text())
                        .map_err(|e| format!("{}: {e}", path.display()))?;
                }
                let cfg = crate::ccd::Config {
                    port: 0,
                    rucc: self.rucc.clone(),
                    work: req.work.join("ccd"),
                    log: Some(std::sync::Arc::new(std::sync::Mutex::new(file))),
                    mix: mix.map(std::sync::Arc::new),
                };
                let port = crate::ccd::start(&cfg)?;
                eprintln!(
                    "r9 build: r9 ccd is on 127.0.0.1:{port} with {}; the calls go to {}",
                    cfg.rucc.display(),
                    log.display()
                );
                Slot::R { port, bringup }
            }
        };
        run(
            &self.release,
            &self.dir,
            req,
            &self.repo_root,
            &slot,
            self.test.as_ref(),
        )
    }
}

/// The records with `origin`.
fn count(records: &[census::Record], origin: &str) -> usize {
    records.iter().filter(|r| r.origin == origin).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_build_and_collect() {
        let c = commands("/sys/src", &Slot::G);
        assert!(c[2].contains("r9-build /sys/src install < /dev/null"));
        assert!(c[2].contains("{while(! test -f /tmp/r9/t1){sleep 60; echo r9-progress"));
        assert!(c[5].contains("awk -v 't='$t0 '$1 >= t+0 {print $3}'"));
        assert!(c[6].starts_with("sha1sum -2 256 /tmp/r9/t0 `{cat /tmp/r9/changed}"));
        assert_eq!(
            c[7],
            "{for(a in `{grep '\\.a.?$' /tmp/r9/changed}){rm -rf /tmp/r9/ar; mkdir /tmp/r9/ar; cd /tmp/r9/ar; ar x $a; sha1sum -2 256 * | awk -v 'a='$a '{print $1 \"\\t\" a \"(\" $2 \")\"}'}} > /tmp/r9/members.sha256; cd / && rm -rf /tmp/r9/ar"
        );
        assert!(c[8].contains("`{grep '\\.[chy]$|/y\\.debug$' /tmp/r9/changed}"));
        assert!(
            c.last()
                .unwrap()
                .ends_with("changed.sha256 members.sha256 gen.tar rucc.err")
        );
        assert!(c.iter().all(|l| !l.contains('\n')));
    }

    #[test]
    fn the_test_runs_in_its_own_note_group_with_a_watchdog() {
        let c = test_command(&Test {
            command: "cd /sys/src/cmd/mothra && mk -n".into(),
            seconds: 30,
        });
        assert!(c.contains("{rfork s; cd /sys/src/cmd/mothra && mk -n} < /dev/null"));
        assert!(c.contains(
            "{sleep 30; echo timeout > /tmp/r9/test.status; echo kill > /proc/$r9t/notepg}"
        ));
        assert!(c.contains("wait $r9t; s=$status;"));
        assert!(c.contains("test -d /proc/$r9w && echo kill > /proc/$r9w/note;"));
        assert!(!c.contains('\n'));
    }

    #[test]
    fn side_r_sends_the_calls_to_the_host() {
        let slot = Slot::R {
            port: 41234,
            bringup: "cc:auto".into(),
        };
        let c = commands("/sys/src/libc", &slot);
        assert!(c[0].ends_with("tar xf $r9in && ip/ipconfig"));
        assert!(c[0].contains("chmod +a /tmp/r9/rucc.err"));
        assert!(c[2].contains(
            "r9ccd='tcp!10.0.2.2!41234' r9bringup='cc:auto' /usr/glenda/r9/bin/r9-build /sys/src/libc install"
        ));
        assert!(!commands("/sys/src", &Slot::G)[2].contains("r9ccd"));
    }
}
