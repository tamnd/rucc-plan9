//! `9h`, the monitor that runs a 9front kernel on the hypervisor of the host (plan 10).

use std::process::ExitCode;
use std::sync::Arc;

use h9_dev::Bus;
use h9_dev::h9;
use h9_dev::h9cons::H9Cons;
use h9_dev::h9disk::H9Disk;
use h9_dev::h9fs::H9Fs;
use h9_dev::pl011::Pl011;
use h9_dev::pl031::Pl031;
use h9_vm::Ram;
use h9_vm::fdt::Fdt;

const USAGE: &str = "usage:
  9h run --kernel FILE [--machine 9h|virt] [--mem MIB] [--cpus N] [--bootargs TEXT]
         [--disk FILE[,ro]]... [--hostfs DIR[,ro]] [--user NAME] [--ini FILE] [--strict]
         [--timeout SECS]
      Run an arm64 kernel of the tree on the hypervisor of the host. FILE is a 7l a.out or the
      U-Boot image of one (9qemu or 9qemu.u). The machine has the layout of the QEMU virt
      machine: RAM at 0x40000000 (--mem MiB, 1024 by default), the device tree at the start of
      the RAM, and the GICv3 of the hypervisor. --machine 9h, the default, is the machine of
      the 9h kernel: its console is a hypercall. --machine virt has a PL011 console in place of
      the hypercall console, so that the qemu kernel of the tree can run. Both have a PL031
      clock. --cpus gives the number of processors, 1 to 16, and 1 by default. The guest starts
      processors 1 and up with PSCI CPU_ON. --bootargs goes in /chosen/bootargs of the device
      tree, where the kernel reads it in place of the plan9.ini. Each --disk is a disk of the 9h
      machine, sdH0 to sdH3 in the kernel, and ,ro makes it read-only. --hostfs serves DIR as
      9P2000 on #Z/hostfs of the 9h kernel, for /mnt/term. Each file has the owner NAME of
      --user, glenda by default. --ini FILE is the plan9.ini,
      which goes at 0x40010000 as U-Boot puts it. An access to an address with no device reads
      as all ones, and --strict makes it stop the session. --timeout ends the session after SECS
      seconds with exit status 2.";

const RAM_BASE: u64 = 0x4000_0000;
const DTB: u64 = RAM_BASE;
const CONFADDR: u64 = RAM_BASE + 0x1_0000;
const KZERO: u64 = 0xffff_ffff_8000_0000;
const UART: u64 = 0x0900_0000;
const UART_SPI: u32 = 33;
const RTC: u64 = 0x0901_0000;
/// The redistributor region of the device tree (0xf60000 bytes) has frames for 123 processors,
/// but the kernel of the tree has MAXMACH 16 (`mem.h` of arm64).
const MAXCPU: u32 = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Machine {
    /// The machine of the 9h kernel.
    H9,
    /// The bring-up machine for the `qemu` kernel of the tree, with a PL011.
    Virt,
}

#[derive(Debug)]
struct Options {
    kernel: String,
    machine: Machine,
    mem: u64,
    cpus: u32,
    bootargs: String,
    ini: Option<String>,
    disks: Vec<(String, bool)>,
    hostfs: Option<(String, bool)>,
    user: String,
    strict: bool,
    timeout: Option<u64>,
}

/// A value with `,ro` at the end is read-only.
fn readonly(v: String) -> (String, bool) {
    match v.strip_suffix(",ro") {
        Some(f) => (f.to_string(), true),
        None => (v, false),
    }
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut o = Options {
        kernel: String::new(),
        machine: Machine::H9,
        mem: 1024,
        cpus: 1,
        bootargs: String::new(),
        ini: None,
        disks: Vec::new(),
        hostfs: None,
        user: "glenda".into(),
        strict: false,
        timeout: None,
    };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let (k, inline) = match a.split_once('=') {
            Some((k, v)) if k.starts_with("--") => (k, Some(v.to_string())),
            _ => (a.as_str(), None),
        };
        let mut val = || {
            inline
                .clone()
                .or_else(|| it.next().cloned())
                .ok_or(format!("{k} needs a value"))
        };
        match k {
            "--kernel" => o.kernel = val()?,
            "--machine" => {
                o.machine = match val()?.as_str() {
                    "9h" => Machine::H9,
                    "virt" => Machine::Virt,
                    m => return Err(format!("--machine {m}: the machines are 9h and virt")),
                };
            }
            "--mem" => o.mem = val()?.parse().map_err(|_| "--mem needs a number of MiB")?,
            "--cpus" => o.cpus = val()?.parse().map_err(|_| "--cpus needs a number")?,
            "--bootargs" => o.bootargs = val()?,
            "--ini" => o.ini = Some(val()?),
            "--disk" => o.disks.push(readonly(val()?)),
            "--hostfs" => o.hostfs = Some(readonly(val()?)),
            "--user" => o.user = val()?,
            "--strict" => o.strict = true,
            "--timeout" => {
                o.timeout = Some(
                    val()?
                        .parse()
                        .map_err(|_| "--timeout needs a number of seconds")?,
                );
            }
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if o.kernel.is_empty() {
        return Err("run needs --kernel".into());
    }
    if !(64..=3072).contains(&o.mem) {
        return Err("--mem must be 64 to 3072 MiB".into());
    }
    if !(1..=MAXCPU).contains(&o.cpus) {
        return Err(format!("--cpus must be 1 to {MAXCPU}"));
    }
    if o.disks.len() > h9_dev::h9disk::MAX_UNITS {
        return Err(format!(
            "the machine has at most {} disks",
            h9_dev::h9disk::MAX_UNITS
        ));
    }
    if o.machine == Machine::Virt && (!o.disks.is_empty() || o.hostfs.is_some()) {
        return Err("--disk and --hostfs are for the 9h machine, not for virt".into());
    }
    if let Some((d, _)) = &o.hostfs
        && !std::path::Path::new(d).is_dir()
    {
        return Err(format!("--hostfs {d}: not a directory"));
    }
    Ok(o)
}

/// The device tree of the machine. The kernel reads `/memory`, `/cpus/cpu*` and `/chosen`.
fn devtree(o: &Options) -> Vec<u8> {
    let mut t = Fdt::new();
    t.begin("");
    t.cells("#address-cells", &[2]);
    t.cells("#size-cells", &[2]);
    t.string("compatible", "linux,dummy-virt");
    t.cells("interrupt-parent", &[1]);
    // The kernel copies /chosen/bootargs over the plan9.ini at CONFADDR when the property is
    // there, so the property is only in the tree when --bootargs is given.
    t.begin("chosen");
    if !o.bootargs.is_empty() {
        t.string("bootargs", &o.bootargs);
    }
    t.end();
    t.begin(&format!("memory@{RAM_BASE:x}"));
    t.string("device_type", "memory");
    t.cells64("reg", &[RAM_BASE, o.mem << 20]);
    t.end();
    t.begin("cpus");
    t.cells("#address-cells", &[1]);
    t.cells("#size-cells", &[0]);
    for n in 0..o.cpus {
        t.begin(&format!("cpu@{n}"));
        t.string("device_type", "cpu");
        t.string("compatible", "arm,arm-v8");
        t.cells("reg", &[n]);
        t.string("enable-method", "psci");
        t.end();
    }
    t.end();
    t.begin("psci");
    t.string("compatible", "arm,psci-1.0");
    t.string("method", "hvc");
    t.end();
    t.begin("intc@8000000");
    t.string("compatible", "arm,gic-v3");
    t.cells("phandle", &[1]);
    t.cells("#interrupt-cells", &[3]);
    t.prop("interrupt-controller", &[]);
    t.cells64("reg", &[0x0800_0000, 0x1_0000, 0x080a_0000, 0xf6_0000]);
    t.end();
    t.begin("timer");
    t.string("compatible", "arm,armv8-timer");
    t.cells("interrupts", &[1, 13, 4, 1, 14, 4, 1, 11, 4, 1, 10, 4]);
    t.end();
    if o.machine == Machine::Virt {
        t.begin(&format!("pl011@{UART:x}"));
        t.string("compatible", "arm,pl011");
        t.cells64("reg", &[UART, 0x1000]);
        t.cells("interrupts", &[0, UART_SPI - 32, 4]);
        t.end();
    }
    t.begin(&format!("pl031@{RTC:x}"));
    t.string("compatible", "arm,pl031");
    t.cells64("reg", &[RTC, 0x1000]);
    t.end();
    t.end();
    t.finish()
}

fn run(o: &Options) -> Result<ExitCode, String> {
    let bytes = std::fs::read(&o.kernel).map_err(|e| format!("{}: {e}", o.kernel))?;
    let image =
        h9_vm::kernel::load(&bytes, KZERO, 0x1_0000).map_err(|e| format!("{}: {e}", o.kernel))?;
    let ram = Arc::new(Ram::new(RAM_BASE, (o.mem << 20) as usize)?);
    for (pa, seg) in &image.segments {
        if !ram.write(*pa, seg) {
            return Err(format!(
                "{}: a segment at {pa:#x} is not in the RAM",
                o.kernel
            ));
        }
    }
    if image.end > CONFADDR
        && image
            .segments
            .iter()
            .any(|(pa, _)| *pa < CONFADDR + 0x1_0000)
    {
        return Err(format!(
            "{}: the kernel overlaps the device tree or plan9.ini",
            o.kernel
        ));
    }
    let dtb = devtree(o);
    if !ram.write(DTB, &dtb) {
        return Err("the device tree does not fit in the RAM".into());
    }
    if let Some(ini) = &o.ini {
        let mut text = std::fs::read(ini).map_err(|e| format!("{ini}: {e}"))?;
        text.truncate(0xffff);
        if !ram.write(CONFADDR, &text) {
            return Err(format!("{ini}: the plan9.ini does not fit in the RAM"));
        }
    }
    let mut bus = Bus::new();
    bus.strict = o.strict;
    match o.machine {
        Machine::Virt => bus.add(
            UART,
            0x1000,
            Box::new(Pl011::new(UART_SPI, Box::new(std::io::stdout()))),
        ),
        Machine::H9 => bus.add_hyper(Box::new(H9Cons::new(
            h9::CONS_SPI,
            Box::new(std::io::stdout()),
        ))),
    }
    if !o.disks.is_empty() {
        let mut d = H9Disk::new();
        for (f, ro) in &o.disks {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(!ro)
                .open(f)
                .map_err(|e| format!("{f}: {e}"))?;
            d.add(file, *ro).map_err(|e| format!("{f}: {e}"))?;
        }
        bus.add_hyper(Box::new(d));
    }
    if let Some((dir, ro)) = &o.hostfs {
        bus.add_hyper(Box::new(H9Fs::new(h9_9p::Options {
            root: dir.into(),
            user: o.user.clone(),
            readonly: *ro,
        })));
    }
    bus.add(RTC, 0x1000, Box::new(Pl031::new()));
    backend::run(ram, bus, o.cpus as usize, image.entry, DTB)
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
mod backend {
    use super::UART;
    use h9_dev::Bus;
    use h9_dev::h9cons::H9Cons;
    use h9_dev::pl011::Pl011;
    use h9_hvf::{Stop, Vm};
    use h9_vm::Ram;
    use std::io::Read;
    use std::process::ExitCode;
    use std::sync::Arc;

    pub fn run(
        ram: Arc<Ram>,
        bus: Bus,
        ncpu: usize,
        entry: u64,
        x0: u64,
    ) -> Result<ExitCode, String> {
        let vm = Vm::new(ram, bus, ncpu)?;
        let m = vm.machine();
        std::thread::spawn(move || {
            let mut b = [0u8; 256];
            let mut stdin = std::io::stdin();
            while let Ok(n @ 1..) = stdin.read(&mut b) {
                m.with_bus(|bus| {
                    if let Some(u) = bus.device::<Pl011>(UART) {
                        b[..n].iter().for_each(|c| u.push(*c));
                    } else if let Some(c) = bus.hyper_device::<H9Cons>() {
                        b[..n].iter().for_each(|x| c.push(*x));
                    }
                });
            }
        });
        match vm.run(entry, x0)? {
            Stop::Off => eprintln!("9h: the guest turned the machine off"),
            Stop::Reset => eprintln!("9h: the guest asked for a reset"),
        }
        Ok(ExitCode::SUCCESS)
    }
}

#[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
mod backend {
    use h9_dev::Bus;
    use h9_vm::Ram;
    use std::process::ExitCode;
    use std::sync::Arc;

    pub fn run(_: Arc<Ram>, _: Bus, _: usize, _: u64, _: u64) -> Result<ExitCode, String> {
        Err("this host has no back end yet: the monitor runs on macOS on arm64 only".into())
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let o = match args.first().map(String::as_str) {
        Some("run") => parse(&args[1..]),
        Some("help" | "-h" | "--help") => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => Err("no command".into()),
    };
    let o = match o {
        Ok(o) => o,
        Err(e) => {
            eprintln!("9h: {e}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if let Some(secs) = o.timeout {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(secs));
            eprintln!("\n9h: the session ran for {secs} seconds, the timeout");
            std::process::exit(2);
        });
    }
    match run(&o) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("9h: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{devtree, parse};

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn the_options_are_read() {
        let o = parse(&args("--kernel 9qemu --mem=512 --strict --timeout 30")).unwrap();
        assert_eq!(
            (o.kernel.as_str(), o.mem, o.strict, o.timeout),
            ("9qemu", 512, true, Some(30))
        );
        assert!(parse(&args("--mem 512")).unwrap_err().contains("--kernel"));
        assert!(parse(&args("--kernel k --mem 9999")).is_err());
        assert_eq!(parse(&args("--kernel k --cpus 4")).unwrap().cpus, 4);
        assert!(parse(&args("--kernel k --cpus 0")).is_err());
        assert!(parse(&args("--kernel k --cpus 17")).is_err());
        let o = parse(&args("--kernel k --disk a.img --disk=b.img,ro")).unwrap();
        assert_eq!(
            o.disks,
            vec![("a.img".to_string(), false), ("b.img".to_string(), true)]
        );
        assert!(parse(&args("--kernel k --machine virt --disk a.img")).is_err());
        let o = parse(&args("--kernel k --hostfs /,ro --user tam")).unwrap();
        assert_eq!(o.hostfs, Some(("/".to_string(), true)));
        assert_eq!(o.user, "tam");
        assert!(parse(&args("--kernel k --hostfs /nonexistent-dir")).is_err());
        assert!(
            parse(&args(
                "--kernel k --disk a --disk b --disk c --disk d --disk e"
            ))
            .is_err()
        );
        assert!(
            parse(&args("--kernel k --what"))
                .unwrap_err()
                .contains("--what")
        );
    }

    #[test]
    fn the_device_tree_has_what_bootargs_c_reads() {
        let o = parse(&args("--kernel k --mem 256 --bootargs console=0")).unwrap();
        let t = devtree(&o);
        let has = |s: &[u8]| t.windows(s.len()).any(|w| w == s);
        assert!(has(b"memory@40000000\0"));
        assert!(has(b"cpu@0\0"));
        assert!(!has(b"cpu@1\0"));
        let o4 = parse(&args("--kernel k --cpus 4")).unwrap();
        let t4 = devtree(&o4);
        assert!(t4.windows(6).any(|w| w == b"cpu@3\0"));
        assert!(has(b"console=0\0"));
        assert!(!has(b"pl011@9000000\0"), "the 9h machine has no PL011");
        let virt = parse(&args("--kernel k --machine virt")).unwrap();
        assert!(devtree(&virt).windows(14).any(|w| w == b"pl011@9000000\0"));
        assert!(parse(&args("--kernel k --machine pc")).is_err());
        let o = parse(&args("--kernel k")).unwrap();
        assert!(
            !devtree(&o).windows(9).any(|w| w == b"bootargs\0"),
            "an empty bootargs would hide the plan9.ini"
        );
        // The reg of /memory: 0x40000000, 256 MiB.
        assert!(has(&[0, 0, 0, 0, 0x40, 0, 0, 0, 0, 0, 0, 0, 0x10, 0, 0, 0]));
    }
}
