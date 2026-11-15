//! `r9 9h boot` and `r9 9h smoke`: boot the 9h kernel in the 9h monitor, run commands on its
//! console, and halt it (plan 10.8 and 12.6). The console of the monitor is its standard input
//! and output, as the serial port of QEMU is for `r9 boot`, so the two commands use the same
//! console driver.
//!
//! The root image is never written. Each boot writes to a copy in the work directory, which is a
//! clone on a file system that can clone a file. A qcow2 image becomes a raw copy through
//! `qemu-img convert`, because the monitor reads raw disks only. The copy is removed after the
//! boot unless `--keep` is given. The console goes to `console.log`, the command of the monitor
//! to `9h.cmd`, and the commands and their results to `steps.json`.

use crate::boot::{self, Outcome};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

/// One boot of the 9h kernel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The 9h monitor.
    pub monitor: PathBuf,
    /// The 9h kernel, a 7l a.out or its U-Boot image.
    pub kernel: PathBuf,
    /// The root image, raw or qcow2. It is sdH0 in the guest.
    pub root: PathBuf,
    /// A raw disk that the guest can only read, attached after the root disk.
    pub disk: Option<PathBuf>,
    /// A directory for `/mnt/term`, which the guest can only read.
    pub hostfs: Option<PathBuf>,
    /// The number of processors.
    pub cpus: u32,
    /// The RAM in MiB.
    pub mem: u64,
    /// The answer to the `bootargs` prompt.
    pub bootargs: String,
    /// The commands to type at the shell prompt.
    pub commands: Vec<String>,
    /// A directory or a tar file for the guest, attached as a raw tar disk.
    pub input: Option<PathBuf>,
    /// The size in MiB of a blank disk for the guest to write a tar archive to.
    pub collect_mib: Option<u64>,
    /// The limit of the whole boot.
    pub timeout: Duration,
    /// The directory for the copy of the root image and the logs.
    pub work: PathBuf,
    /// Keep the copy of the root image after the boot.
    pub keep: bool,
    /// Stop the session at an access to an address with no device.
    pub strict: bool,
}

/// The bootargs of the kernel: the console is the uart of 9h, which is `/dev/eia0`.
pub const CONSOLE: &str = "console=0";

/// The answer to the `bootargs` prompt when none is given: the file system of the root disk.
pub const BOOTARGS: &str = "local!/dev/sdH0/fs";

/// The copy of the root image in the work directory.
#[must_use]
pub fn root_copy(work: &Path) -> PathBuf {
    work.join("root.img")
}

/// Whether `path` is a qcow2 image.
fn is_qcow2(path: &Path) -> Result<bool, String> {
    use std::io::Read;
    let mut magic = [0u8; 4];
    let mut f = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(f.read_exact(&mut magic).is_ok() && magic == *b"QFI\xfb")
}

/// Make the copy of the root image that the boot writes to.
fn copy_root(root: &Path, copy: &Path) -> Result<(), String> {
    let _ = std::fs::remove_file(copy);
    if is_qcow2(root)? {
        let status = Command::new("qemu-img")
            .args(["convert", "-O", "raw"])
            .arg(root)
            .arg(copy)
            .status()
            .map_err(|e| format!("cannot run qemu-img: {e}"))?;
        if !status.success() {
            return Err(format!("qemu-img could not convert {}", root.display()));
        }
        return Ok(());
    }
    // std::fs::copy clones the file on APFS and on Btrfs, so a big image costs no space.
    std::fs::copy(root, copy)
        .map(|_| ())
        .map_err(|e| format!("cannot copy {}: {e}", root.display()))
}

/// The command of the monitor. `disks` are the raw tar disks, after the root disk and `disk`.
/// The snarf buffer is in the monitor, so a boot does not change the host clipboard. The kernel
/// gets `console=0` as its bootargs, because `bootrc` starts `kbdfs` on `/dev/eia$console`, and
/// with no `console` the console has no input.
#[must_use]
pub fn command(req: &Request, disks: &[PathBuf]) -> Vec<String> {
    let mut c = vec![
        req.monitor.display().to_string(),
        "run".into(),
        "--kernel".into(),
        req.kernel.display().to_string(),
        "--cpus".into(),
        req.cpus.to_string(),
        "--mem".into(),
        req.mem.to_string(),
        "--snarf".into(),
        "memory".into(),
        "--headless".into(),
        "--bootargs".into(),
        CONSOLE.into(),
        "--disk".into(),
        root_copy(&req.work).display().to_string(),
    ];
    if req.strict {
        c.push("--strict".into());
    }
    if let Some(d) = &req.disk {
        c.push("--disk".into());
        c.push(format!("{},ro", d.display()));
    }
    for d in disks {
        c.push("--disk".into());
        c.push(d.display().to_string());
    }
    if let Some(d) = &req.hostfs {
        c.push("--hostfs".into());
        c.push(format!("{},ro", d.display()));
    }
    c
}

/// Boot the 9h kernel and run the commands.
pub fn run(req: &Request) -> Result<Outcome, String> {
    std::fs::create_dir_all(&req.work).map_err(|e| format!("{}: {e}", req.work.display()))?;
    let mut disks = Vec::new();
    let mut commands = req.commands.clone();
    // The disk of --disk has a file system or not, so the setup commands count only the tar
    // disks, which come last.
    boot::add_tar_disks(
        &req.work,
        req.input.as_deref(),
        req.collect_mib,
        &mut disks,
        &mut commands,
    )?;
    copy_root(&req.root, &root_copy(&req.work))?;

    let mc = command(req, &disks);
    std::fs::write(req.work.join("9h.cmd"), format!("{}\n", mc.join(" ")))
        .map_err(|e| format!("{}: {e}", req.work.display()))?;
    let log_path = req.work.join("console.log");
    let mut log =
        std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let stderr = std::fs::File::create(req.work.join("9h.stderr"))
        .map_err(|e| format!("{}: {e}", req.work.display()))?;
    let mut child = Command::new(&mc[0])
        .args(&mc[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", mc[0]))?;
    let driven = boot::drive(
        &mut child,
        &mut log,
        &req.bootargs,
        commands,
        req.timeout,
        || {},
    );
    let collected = match req.collect_mib {
        Some(_) => boot::collect(&req.work),
        None => Ok(()),
    };
    if !req.keep {
        let _ = std::fs::remove_file(root_copy(&req.work));
        let _ = std::fs::remove_file(crate::disk::in_disk(&req.work));
    }
    let outcome = driven?.outcome();
    collected?;
    boot::write_steps(&req.work.join("steps.json"), &outcome)?;
    Ok(outcome)
}

/// The monitor that `r9 9h boot` runs when `--monitor` is not given: the release build in the
/// repository, or `9h` on the path.
fn default_monitor(repo: &crate::repo::Repo) -> PathBuf {
    let built = repo
        .root
        .join("9h")
        .join("monitor")
        .join("target")
        .join("release")
        .join("9h");
    if built.is_file() {
        built
    } else {
        PathBuf::from("9h")
    }
}

/// The request of `r9 9h boot` and `r9 9h smoke`. `smoke` runs `smoke/suite.rc` with the
/// `guest` directory as the input, in place of `smoke/boot.rc` with no input.
pub fn request_from(
    repo: &crate::repo::Repo,
    args: &crate::cli::Args,
    smoke: bool,
) -> Result<Request, String> {
    let need = |k: &str| {
        args.get(k)
            .map(PathBuf::from)
            .ok_or_else(|| format!("give --{k}"))
    };
    let kernel = need("kernel")?;
    let root = need("root")?;
    for f in [&kernel, &root] {
        if !f.is_file() {
            return Err(format!("{} is not a file", f.display()));
        }
    }
    let number = |k: &str, default: u64| -> Result<u64, String> {
        args.get(k).map_or(Ok(default), |v| {
            v.parse().map_err(|_| format!("--{k} {v}: give a number"))
        })
    };
    let script = if smoke { "suite.rc" } else { "boot.rc" };
    let run = args
        .get("run")
        .map_or_else(|| repo.root.join("smoke").join(script), PathBuf::from);
    let commands = boot::read_commands(
        &std::fs::read_to_string(&run).map_err(|e| format!("{}: {e}", run.display()))?,
    );
    let input = args
        .get("in")
        .map(PathBuf::from)
        .or_else(|| smoke.then(|| repo.root.join("guest")));
    let cpus = u32::try_from(number("cpus", 1)?).map_err(|_| "--cpus is too big")?;
    let work = args.get("out").map_or_else(
        || {
            repo.root
                .join("work")
                .join("9h")
                .join(if smoke { "smoke" } else { "boot" })
        },
        PathBuf::from,
    );
    Ok(Request {
        monitor: args
            .get("monitor")
            .map_or_else(|| default_monitor(repo), PathBuf::from),
        kernel,
        root,
        disk: args.get("disk").map(PathBuf::from),
        hostfs: args.get("hostfs").map(PathBuf::from),
        cpus,
        mem: number("mem", 1024)?,
        bootargs: args.get("bootargs").unwrap_or(BOOTARGS).to_string(),
        commands,
        input,
        collect_mib: args
            .get("collect")
            .map(|m| {
                m.parse()
                    .map_err(|_| format!("--collect {m}: give a size in MiB"))
            })
            .transpose()?,
        timeout: Duration::from_secs(number("timeout", 3600)?),
        work,
        keep: args.has("keep"),
        strict: args.has("strict"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::console::Phase;

    fn request(work: &Path) -> Request {
        Request {
            monitor: "/b/9h".into(),
            kernel: "/k/99h.u".into(),
            root: "/i/root.img".into(),
            disk: Some("/i/src.img".into()),
            hostfs: Some("/home/glenda".into()),
            cpus: 2,
            mem: 2048,
            bootargs: BOOTARGS.into(),
            commands: vec![],
            input: None,
            collect_mib: None,
            timeout: Duration::from_secs(60),
            work: work.into(),
            keep: false,
            strict: true,
        }
    }

    #[test]
    fn the_monitor_command() {
        let c = command(&request(Path::new("/w")), &["/w/in.tar".into()]).join(" ");
        assert_eq!(
            c,
            "/b/9h run --kernel /k/99h.u --cpus 2 --mem 2048 --snarf memory --headless \
             --bootargs console=0 --disk /w/root.img --strict --disk /i/src.img,ro --disk /w/in.tar --hostfs /home/glenda,ro"
        );
    }

    /// A monitor in Perl that acts as the console of the guest: it echoes each byte, asks for
    /// the bootargs and the user, runs each command in the shell, and gives the status as rc
    /// does, empty when the command succeeded.
    const FAKE: &str = r#"#!/usr/bin/perl
$| = 1;
sub line { my $l = ""; while (sysread(STDIN, my $c, 1)) { print $c; return $l if $c eq "\n"; $l .= $c; } exit 0; }
print "args @ARGV\n";
print "bootargs is (tcp, tls, il, local!device)[] ";
my $b = line();
print "user[glenda]: ";
line();
while (1) {
    print "term% ";
    my $l = line();
    if ($l eq "fshalt") { print "syncing... done halting\n"; sleep 60; exit 0; }
    my ($c) = $l =~ /^(.*); echo R9-STATUS/;
    print `$c 2>&1`;
    print "R9-STATUS ", ($? ? "fail" : ""), "\n";
}
"#;

    #[cfg(unix)]
    #[test]
    fn a_boot_runs_the_commands_and_halts() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("r9-h9boot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let monitor = dir.join("9h");
        std::fs::write(&monitor, FAKE).unwrap();
        std::fs::set_permissions(&monitor, std::fs::Permissions::from_mode(0o755)).unwrap();
        let root = dir.join("root.raw");
        std::fs::write(&root, vec![0u8; 4096]).unwrap();
        let work = dir.join("work");
        let mut req = request(&work);
        req.monitor = monitor;
        req.root = root;
        req.disk = None;
        req.hostfs = None;
        req.commands = vec!["echo hi".into(), "false".into(), "echo end".into()];
        let o = run(&req).unwrap();
        assert_eq!(o.phase, Phase::Halted);
        assert!(!o.passed, "false fails");
        let status: Vec<_> = o.steps.iter().map(|s| s.status.clone()).collect();
        assert_eq!(
            status,
            [
                Some(String::new()),
                Some("fail".into()),
                Some(String::new())
            ]
        );
        assert_eq!(o.steps[0].output, ["hi"]);
        let console = std::fs::read_to_string(work.join("console.log")).unwrap();
        assert!(console.contains(&format!("{BOOTARGS}\n")), "{console}");
        assert!(console.contains("--disk "), "{console}");
        assert!(!root_copy(&work).exists(), "the copy is removed");
        assert!(work.join("steps.json").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
