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
) -> Result<(Summary, String), String> {
    req.commands = commands(dir, slot);
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

/// `r9 build` from the command line.
pub fn command(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
) -> Result<(Summary, String), String> {
    let r = match args.get("side") {
        Some("G" | "g") => false,
        Some("R" | "r") => true,
        _ => return Err("give --side G or --side R".into()),
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
            if r { "R" } else { "G" }
        ));
    }
    let slot = if r {
        let log = req.work.join("ccd.log");
        std::fs::create_dir_all(&req.work).map_err(|e| format!("{}: {e}", req.work.display()))?;
        let file = std::fs::File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        let rucc = args.get("rucc").map_or_else(
            || std::env::var_os("R9_RUCC").map_or_else(|| "rucc".into(), Into::into),
            Into::into,
        );
        let cfg = crate::ccd::Config {
            port: 0,
            rucc,
            work: req.work.join("ccd"),
            log: Some(std::sync::Arc::new(std::sync::Mutex::new(file))),
        };
        let port = crate::ccd::start(&cfg)?;
        eprintln!(
            "r9 build: r9 ccd is on 127.0.0.1:{port} with {}; the calls go to {}",
            cfg.rucc.display(),
            log.display()
        );
        Slot::R {
            port,
            bringup: std::env::var("R9_BRINGUP").unwrap_or_else(|_| "cc:auto".into()),
        }
    } else {
        Slot::G
    };
    run(
        &release,
        args.get("dir").unwrap_or("/sys/src"),
        req,
        &repo.root,
        &slot,
    )
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
