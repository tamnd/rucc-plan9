//! `r9 boot`: boot a release image under QEMU, run commands on its console, and halt it.
//!
//! The image in the cache is never written. Each boot writes to a fresh qcow2 overlay in the
//! work directory, and the overlay is removed after the boot unless `--keep` is given. The
//! QEMU machine comes from `rows.toml`. The console goes to `console.log`, and the commands and
//! their results go to `steps.json`.

use crate::console::{Action, Lines, Phase, Session, Step};
use r9_pins::rows::Row;
use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// One boot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    /// The row name, which is the `objtype`.
    pub objtype: String,
    /// The row.
    pub row: Row,
    /// The unpacked release image.
    pub base: PathBuf,
    /// The directory for the overlay and the logs.
    pub work: PathBuf,
    /// `kvm`, `hvf` or `tcg`.
    pub accel: String,
    /// The u-boot binary, for a row whose firmware is `u-boot`.
    pub firmware: Option<PathBuf>,
    /// The answer to the `bootargs` prompt, or empty for the default.
    pub bootargs: String,
    /// The commands to type at the shell prompt.
    pub commands: Vec<String>,
    /// More raw disks, attached after the root disk in this order.
    pub disks: Vec<PathBuf>,
    /// The limit of the whole boot.
    pub timeout: Duration,
    /// Keep the overlay after the boot.
    pub keep: bool,
}

/// What came of a boot.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// The commands and their results.
    pub steps: Vec<Step>,
    /// Where the guest was at the end.
    pub phase: Phase,
    /// The panic line, if any.
    pub panic: Option<String>,
    /// Whether the shell prompt was seen.
    pub ready: bool,
    /// Whether the limit ended the boot.
    pub timed_out: bool,
    /// The time the boot took.
    pub seconds: f64,
    /// Whether every command ran and succeeded, and the guest halted.
    pub passed: bool,
}

/// The overlay file of a boot.
#[must_use]
pub fn overlay(req: &Request) -> PathBuf {
    req.work.join("root.qcow2")
}

/// The command that makes the overlay.
#[must_use]
pub fn overlay_command(req: &Request) -> Vec<String> {
    vec![
        "qemu-img".into(),
        "create".into(),
        "-q".into(),
        "-f".into(),
        "qcow2".into(),
        "-F".into(),
        "qcow2".into(),
        "-b".into(),
        req.base.display().to_string(),
        overlay(req).display().to_string(),
    ]
}

/// The QEMU command line. The console is the first serial port on standard input and output.
#[must_use]
pub fn qemu_command(req: &Request) -> Vec<String> {
    let row = &req.row;
    let hw = req.accel != "tcg";
    let cpu = if hw { &row.cpu_hw } else { &row.cpu_tcg };
    let mut c: Vec<String> = vec![
        row.qemu.clone(),
        "-machine".into(),
        format!("{},accel={}", row.machine, req.accel),
        "-cpu".into(),
        cpu.clone(),
        "-smp".into(),
        row.cpus.to_string(),
        "-m".into(),
        format!("{}G", row.memory_gb),
        "-nodefaults".into(),
        "-display".into(),
        "none".into(),
        "-serial".into(),
        "stdio".into(),
    ];
    if let Some(fw) = &req.firmware {
        c.push("-bios".into());
        c.push(fw.display().to_string());
    }
    // 9front's arm64 kernel has only the virtio 1.0 driver, so the devices drop the legacy
    // interface there.
    let modern = if row.qemu.ends_with("aarch64") {
        ",disable-legacy=on"
    } else {
        ""
    };
    let device = |id: &str| match row.disk.as_str() {
        "ahci" => format!("ide-hd,drive={id}"),
        _ => format!("virtio-blk-pci,drive={id}{modern}"),
    };
    c.push("-drive".into());
    c.push(format!(
        "file={},if=none,id=root,format=qcow2,cache=unsafe",
        overlay(req).display()
    ));
    c.push("-device".into());
    c.push(device("root"));
    for (i, disk) in req.disks.iter().enumerate() {
        let id = format!("d{}", i + 1);
        c.push("-drive".into());
        c.push(format!(
            "file={},if=none,id={id},format=raw",
            disk.display()
        ));
        c.push("-device".into());
        c.push(device(&id));
    }
    c
}

/// The accelerator for a row on this host: KVM on Linux when `/dev/kvm` can be opened, HVF on
/// macOS for a guest of the host's architecture, and TCG otherwise.
#[must_use]
pub fn accel_for(row: &Row) -> String {
    let same_arch = row.rucc_arch == std::env::consts::ARCH;
    if same_arch
        && cfg!(target_os = "linux")
        && std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/kvm")
            .is_ok()
    {
        return "kvm".into();
    }
    if same_arch && cfg!(target_os = "macos") {
        return "hvf".into();
    }
    "tcg".into()
}

/// Find u-boot for QEMU's arm64 `virt` machine: `R9_UBOOT`, or the path of Ubuntu's
/// `u-boot-qemu` package.
#[must_use]
pub fn find_uboot() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("R9_UBOOT").map(PathBuf::from) {
        return Some(p);
    }
    ["/usr/lib/u-boot/qemu_arm64/u-boot.bin"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

/// Boot the guest and run the commands.
pub fn run(req: &Request) -> Result<Outcome, String> {
    std::fs::create_dir_all(&req.work).map_err(|e| format!("{}: {e}", req.work.display()))?;
    let _ = std::fs::remove_file(overlay(req));
    let oc = overlay_command(req);
    let status = Command::new(&oc[0])
        .args(&oc[1..])
        .status()
        .map_err(|e| format!("cannot run qemu-img: {e}"))?;
    if !status.success() {
        return Err(format!("{} failed", oc.join(" ")));
    }

    let qc = qemu_command(req);
    std::fs::write(req.work.join("qemu.cmd"), format!("{}\n", qc.join(" ")))
        .map_err(|e| format!("{}: {e}", req.work.display()))?;
    let log_path = req.work.join("console.log");
    let mut log =
        std::fs::File::create(&log_path).map_err(|e| format!("{}: {e}", log_path.display()))?;
    let stderr = std::fs::File::create(req.work.join("qemu.stderr"))
        .map_err(|e| format!("{}: {e}", req.work.display()))?;
    let mut child = Command::new(&qc[0])
        .args(&qc[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(stderr)
        .spawn()
        .map_err(|e| format!("cannot run {}: {e}", qc[0]))?;
    let mut stdin = child.stdin.take().ok_or("no stdin for QEMU")?;
    let mut stdout = child.stdout.take().ok_or("no stdout for QEMU")?;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    let reader = std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        while let Ok(n) = stdout.read(&mut buf) {
            if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                break;
            }
        }
    });

    let start = Instant::now();
    let mut session = Session::new(&req.bootargs, req.commands.clone());
    let mut lines = Lines::default();
    let mut timed_out = false;
    let mut halted_at: Option<Instant> = None;
    loop {
        if start.elapsed() > req.timeout {
            timed_out = true;
            break;
        }
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(bytes) => {
                let _ = log.write_all(&bytes);
                for line in lines.push(&bytes) {
                    session.line(&line);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        match session.phase {
            Phase::Panicked => break,
            Phase::Halted => {
                // fshalt leaves the machine running; give the console a moment, then stop it.
                let at = *halted_at.get_or_insert_with(Instant::now);
                if at.elapsed() > Duration::from_secs(2) {
                    break;
                }
            }
            _ => {}
        }
        if let Action::Type(text) = session.prompt(&lines.partial()) {
            lines.answered();
            // The console reads one character at a time; a short pause between characters keeps
            // the 16550's FIFO from overflowing under TCG.
            for chunk in text.as_bytes().chunks(16) {
                stdin
                    .write_all(chunk)
                    .and_then(|()| stdin.flush())
                    .map_err(|e| format!("cannot type into QEMU: {e}"))?;
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    if !req.keep {
        let _ = std::fs::remove_file(overlay(req));
    }
    let passed = session.passed() && session.phase == Phase::Halted && !timed_out;
    let outcome = Outcome {
        steps: session.steps.clone(),
        phase: session.phase,
        panic: session.panic.clone(),
        ready: session.ready,
        timed_out,
        seconds: start.elapsed().as_secs_f64(),
        passed,
    };
    write_steps(&req.work.join("steps.json"), &outcome)?;
    Ok(outcome)
}

fn write_steps(path: &Path, o: &Outcome) -> Result<(), String> {
    let steps: Vec<serde_json::Value> = o
        .steps
        .iter()
        .map(|s| {
            serde_json::json!({
                "command": s.command,
                "status": s.status,
                "output": s.output,
            })
        })
        .collect();
    let value = serde_json::json!({
        "phase": format!("{:?}", o.phase).to_lowercase(),
        "panic": o.panic,
        "ready": o.ready,
        "timed_out": o.timed_out,
        "seconds": (o.seconds * 10.0).round() / 10.0,
        "passed": o.passed,
        "steps": steps,
    });
    let text = serde_json::to_string_pretty(&value).map_err(|e| e.to_string())?;
    std::fs::write(path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))
}

/// The report `r9 boot` prints.
#[must_use]
pub fn summary(o: &Outcome, work: &Path) -> String {
    let mut out = String::new();
    for s in &o.steps {
        let result = match s.status.as_deref() {
            Some("") => "ok".to_string(),
            Some(status) => format!("FAIL {status}"),
            None => "NO ANSWER".to_string(),
        };
        let _ = writeln!(out, "{result:<8} {}", s.command);
    }
    if let Some(p) = &o.panic {
        let _ = writeln!(out, "panic: {p}");
    }
    let end = if o.timed_out {
        "timed out".to_string()
    } else {
        format!("{:?}", o.phase).to_lowercase()
    };
    let _ = writeln!(
        out,
        "{} in {:.1}s; {}; console in {}",
        if o.passed { "passed" } else { "failed" },
        o.seconds,
        end,
        work.join("console.log").display()
    );
    out
}

/// Read a command file: one command per line, with `#` comments and blank lines left out.
#[must_use]
pub fn read_commands(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

/// The request of `r9 boot NAME --row OBJTYPE ...`. With no name, the newest release in the
/// `current` set is booted. The image must be in the cache already; `r9 fetch` puts it there.
pub fn request_from(repo: &crate::repo::Repo, args: &crate::cli::Args) -> Result<Request, String> {
    use r9_pins::pins::PinsFile;
    let file = PinsFile::load(&repo.root)?.ok_or("there is no pins.toml; run r9 sets --write")?;
    let name = match &args.target {
        Some(n) => n.clone(),
        None => file
            .release
            .iter()
            .rev()
            .find(|(_, r)| r.sets.iter().any(|s| s == r9_pins::sets::CURRENT))
            .map(|(n, _)| n.clone())
            .ok_or("pins.toml has no release in the current set")?,
    };
    let release = file
        .release
        .get(&name)
        .ok_or_else(|| format!("pins.toml has no release {name}"))?;
    let objtype = args.get("row").unwrap_or("amd64").to_string();
    let rows = r9_pins::rows::Rows::load(&repo.root)?;
    let row = rows.get(&objtype)?.clone();
    let file_name = format!("9front-{name}.{}.gz", row.image);
    let image = release
        .image
        .iter()
        .find(|i| i.file == file_name)
        .ok_or_else(|| format!("release {name} has no {file_name}"))?;
    let base = r9_pins::fetch::image_path(image, &repo.cache);
    if !base.is_file() {
        return Err(format!(
            "{} is not in the cache; run r9 fetch {name} --row {objtype}",
            base.display()
        ));
    }
    let accel = args
        .get("accel")
        .map_or_else(|| accel_for(&row), str::to_string);
    if !matches!(accel.as_str(), "kvm" | "hvf" | "tcg") {
        return Err(format!("--accel {accel}: give kvm, hvf or tcg"));
    }
    let firmware = match row.firmware.as_str() {
        "u-boot" => Some(
            args.get("firmware")
                .map(PathBuf::from)
                .or_else(find_uboot)
                .ok_or("this row boots with u-boot; give --firmware or set R9_UBOOT")?,
        ),
        _ => args.get("firmware").map(PathBuf::from),
    };
    let run = args
        .get("run")
        .map_or_else(|| repo.root.join("smoke").join("boot.rc"), PathBuf::from);
    let commands = read_commands(
        &std::fs::read_to_string(&run).map_err(|e| format!("{}: {e}", run.display()))?,
    );
    let hours = u64::from(row.run_limit_hours);
    let timeout = match args.get("timeout") {
        Some(t) => Duration::from_secs(
            t.parse()
                .map_err(|_| format!("--timeout {t}: give a number of seconds"))?,
        ),
        None => Duration::from_secs(hours * 3600),
    };
    let work = args.get("out").map_or_else(
        || {
            repo.root
                .join("work")
                .join("boot")
                .join(format!("{name}-{objtype}"))
        },
        PathBuf::from,
    );
    Ok(Request {
        objtype,
        row,
        base,
        work,
        accel,
        firmware,
        bootargs: args.get("bootargs").unwrap_or_default().to_string(),
        commands,
        disks: Vec::new(),
        timeout,
        keep: args.has("keep"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(qemu: &str, disk: &str) -> Row {
        Row {
            from: "P0".into(),
            rucc_arch: "x86_64".into(),
            letter: "6".into(),
            image: "amd64.qcow2".into(),
            kernel: "9pc64".into(),
            qemu: qemu.into(),
            machine: "q35".into(),
            cpu_hw: "host".into(),
            cpu_tcg: "max".into(),
            cpus: 4,
            memory_gb: 4,
            firmware: "bios".into(),
            disk: disk.into(),
            run_limit_hours: 4,
        }
    }

    fn request(row: Row, accel: &str) -> Request {
        Request {
            objtype: "amd64".into(),
            row,
            base: "/c/images/9front-11952.amd64.qcow2".into(),
            work: "/w".into(),
            accel: accel.into(),
            firmware: None,
            bootargs: String::new(),
            commands: vec![],
            disks: vec!["/w/data.tar".into()],
            timeout: Duration::from_secs(60),
            keep: false,
        }
    }

    #[test]
    fn the_amd64_command() {
        let c = qemu_command(&request(row("qemu-system-x86_64", "virtio"), "kvm")).join(" ");
        assert!(c.starts_with("qemu-system-x86_64 -machine q35,accel=kvm -cpu host -smp 4 -m 4G"));
        assert!(c.contains("-serial stdio"));
        assert!(c.contains("file=/w/root.qcow2,if=none,id=root,format=qcow2"));
        assert!(c.contains("-device virtio-blk-pci,drive=root -drive file=/w/data.tar,if=none,id=d1,format=raw -device virtio-blk-pci,drive=d1"));
        let c = qemu_command(&request(row("qemu-system-x86_64", "ahci"), "tcg")).join(" ");
        assert!(c.contains("-cpu max"));
        assert!(c.contains("-device ide-hd,drive=root"));
    }

    #[test]
    fn the_arm64_command_drops_legacy_virtio() {
        let mut req = request(row("qemu-system-aarch64", "virtio"), "hvf");
        req.firmware = Some("/u-boot.bin".into());
        let c = qemu_command(&req).join(" ");
        assert!(c.contains("-bios /u-boot.bin"));
        assert!(c.contains("virtio-blk-pci,drive=root,disable-legacy=on"));
    }

    #[test]
    fn the_overlay_is_backed_by_the_image() {
        let c = overlay_command(&request(row("q", "virtio"), "tcg")).join(" ");
        assert_eq!(
            c,
            "qemu-img create -q -f qcow2 -F qcow2 -b /c/images/9front-11952.amd64.qcow2 /w/root.qcow2"
        );
    }

    #[test]
    fn command_files() {
        let c = read_commands("# smoke\ncat /dev/sysname\n\n  ls /bin  \n");
        assert_eq!(c, vec!["cat /dev/sysname", "ls /bin"]);
    }
}
