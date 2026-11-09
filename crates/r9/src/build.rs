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
//! Side R, with rucc in the compiler slot, comes with `r9 ccd` at P1.

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
    "gen.tar",
];

/// The commands that the guest runs to build `dir` with `mk install`. Each command is one line
/// at the shell prompt.
#[must_use]
pub fn commands(dir: &str) -> Vec<String> {
    let w = "/tmp/r9";
    vec![
        format!("mkdir -p /tmp/r9in /usr/glenda/r9/bin {w} && cd /tmp/r9in && tar xf $r9in"),
        "cp /tmp/r9in/r9-build /usr/glenda/r9/bin/ && cd /tmp/r9in/r9-cc && mk install".into(),
        // A loop in the background prints a progress line to the console every minute: the
        // time, the lines of the log, its last line and the memory in use. The console log on
        // the host keeps it when the guest stops before its file server writes the log out.
        format!(
            "date -n > {w}/t0; {{while(! test -f {w}/t1){{sleep 60; echo r9-progress `{{date -n}} `{{wc -l < {w}/mk.log}} `{{tail -1 {w}/mk.log}}; grep memory /dev/swap}}}} & r9p=$apid; /usr/glenda/r9/bin/r9-build {dir} install < /dev/null > {w}/mk.log >[2=1]; s=$status; date -n > {w}/t1; echo kill > /proc/$r9p/note; echo $s > {w}/status; test -z $s"
        ),
        format!("tail -20 {w}/mk.log"),
        format!("walk -f -e msp {dir} /$objtype/bin /$objtype/lib > {w}/walk"),
        format!(
            "t0=`{{cat {w}/t0}}; awk -v 't='$t0 '$1 >= t+0 {{print $3}}' {w}/walk > {w}/changed"
        ),
        // t0 is always named, so that an empty list does not make sha1sum read its input.
        format!("sha1sum -2 256 {w}/t0 `{{cat {w}/changed}} > {w}/changed.sha256"),
        format!("tar cf {w}/gen.tar {w}/t0 `{{grep '\\.[chy]$' {w}/changed}}"),
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
    /// `G` for kencc.
    pub side: String,
    /// The tree that was built.
    pub dir: String,
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
) -> Result<(Summary, String), String> {
    req.commands = commands(dir);
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
        side: "G".into(),
        dir: dir.to_string(),
        accel: req.accel.clone(),
        passed: t1 > 0 && status.is_empty(),
        status,
        build_seconds: t1.saturating_sub(t0),
        boot_seconds: outcome.seconds,
        calls: c.calls,
        failed_calls: c.failed.len(),
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
    report.push_str(&c.report(census::Side::G));
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
    match args.get("side") {
        Some("G" | "g") => {}
        Some("R" | "r") => return Err("side R needs r9 ccd, which comes at P1".into()),
        _ => return Err("give --side G".into()),
    }
    let mut req = boot::request_from(repo, args)?;
    let release = req
        .base
        .file_name()
        .and_then(|f| f.to_str())
        .and_then(|f| f.strip_prefix("9front-"))
        .and_then(|f| f.split('.').next())
        .unwrap_or_default()
        .to_string();
    if args.get("out").is_none() {
        req.work = repo
            .root
            .join("work")
            .join("build")
            .join(format!("{release}-{}-G", req.objtype));
    }
    run(
        &release,
        args.get("dir").unwrap_or("/sys/src"),
        req,
        &repo.root,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_build_and_collect() {
        let c = commands("/sys/src");
        assert!(c[2].contains("r9-build /sys/src install < /dev/null"));
        assert!(c[2].contains("{while(! test -f /tmp/r9/t1){sleep 60; echo r9-progress"));
        assert!(c[5].contains("awk -v 't='$t0 '$1 >= t+0 {print $3}'"));
        assert!(c[6].starts_with("sha1sum -2 256 /tmp/r9/t0 `{cat /tmp/r9/changed}"));
        assert!(c[7].contains("`{grep '\\.[chy]$' /tmp/r9/changed}"));
        assert!(c.last().unwrap().ends_with("changed.sha256 gen.tar"));
        assert!(c.iter().all(|l| !l.contains('\n')));
    }
}
