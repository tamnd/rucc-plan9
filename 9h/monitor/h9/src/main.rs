//! `9h`, the monitor that runs a 9front kernel on the hypervisor of the host (plan 10).

use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::Arc;

use h9_dev::acpipm::{self, AcpiPm};
use h9_dev::cmos::{self, Cmos};
use h9_dev::h9;
use h9_dev::h9cons::H9Cons;
use h9_dev::h9ctl::H9Ctl;
use h9_dev::h9disk::H9Disk;
use h9_dev::h9fs::H9Fs;
use h9_dev::h9mouse::{self, H9Mouse};
use h9_dev::h9screen::{self, H9Screen};
use h9_dev::h9snarf::{Clipboard, H9Snarf, Host, Memory};
use h9_dev::pcinone::{self, PciNone};
use h9_dev::pl011::Pl011;
use h9_dev::pl031::Pl031;
use h9_dev::{Bus, PORTS};
use h9_vm::Ram;
use h9_vm::fdt::Fdt;
use h9_vm::{acpi, multiboot};

// Only a host with a back end runs a script.
#[cfg_attr(
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )),
    allow(dead_code)
)]
mod script;

use script::{Step, Tee, Transcript};

const USAGE: &str = "usage:
  9h run --kernel FILE [--machine 9h|virt] [--mem MIB] [--cpus N] [--bootargs TEXT]
         [--disk FILE[,ro]]... [--hostfs DIR[,ro]] [--user NAME] [--snarf host|memory]
         [--ini FILE] [--strict] [--timeout SECS] [--script FILE] [--headless]
         [--screen WxH|none] [--snap FILE] [--control PATH]
      Run a kernel of the tree on the hypervisor of the host: an arm64 kernel on HVF on macOS
      on arm64 or on KVM on Linux on arm64, and an amd64 kernel on KVM on Linux on x86-64.
      KVM on arm64 needs Linux 6.4 or later, for the SMCCC filter. On arm64, FILE is a 7l a.out
      or the U-Boot image of one (9qemu or 9qemu.u). The machine has the layout of the QEMU virt
      machine: RAM at 0x40000000 (--mem MiB, 1024 by default), the device tree at the start of
      the RAM, and the GICv3 of the hypervisor. --machine 9h, the default, is the machine of
      the 9h kernel: its console is a hypercall. --machine virt has a PL011 console in place of
      the hypercall console, so that the qemu kernel of the tree can run. Both have a PL031
      clock. --cpus gives the number of processors, 1 to 16, and 1 by default. The guest starts
      processors 1 and up with PSCI CPU_ON. --bootargs goes in /chosen/bootargs of the device
      tree, where the kernel reads it in place of the plan9.ini. Each --disk is a disk of the 9h
      machine, sdH0 to sdH3 in the kernel, and ,ro makes it read-only. --hostfs serves DIR as
      9P2000 on #Z/hostfs of the 9h kernel, for /mnt/term. Each file has the owner NAME of
      --user, glenda by default. #Z/snarf of the 9h kernel is the host
      clipboard with --snarf host, the default, and a buffer of the monitor with --snarf memory.
      #Z/ctl takes the requests quit and title TEXT. --ini FILE is the plan9.ini, which goes at 0x40010000 as U-Boot puts it. An access to an address with no device reads
      as all ones, and --strict makes it stop the session. --timeout ends the session after SECS
      seconds with exit status 2. --script FILE drives the console with the steps of FILE: wait
      TEXT, send TEXT, timeout SECS, pause SECS, mouse X Y [BUTTONS], click X Y [BUTTON], quiet [MS] [SECS], snap FILE [X0 Y0 X1 Y1] and quit. The script owns the input, so the monitor does
      not read its own. The exit status is 0 when the script passes and 3 when it fails.
      --control PATH makes a Unix socket at PATH, on which a program of the host gives the steps send, mouse, click, pause, quiet and snap, one line each. The monitor answers each line with ok or error. --headless opens no window. On macOS on arm64, and on Linux when DISPLAY is set, a 9h
      session with a screen shows the screen in a window, which gives the keyboard and the mouse of the host to the machine. On
      Linux, the window is an X11 window. With no DISPLAY, a session on Linux is headless. The 9h machine has a screen of --screen WxH, 1024x768 by default, in the RAM of the
      guest. With --screen none, the machine has no screen, so the profile of the guest does not start rio. --snap FILE writes the screen as a PNG file at the end of the session.
      On amd64, FILE is a 6l kernel with a multiboot header (9pc64). The machine is a PC with
      RAM from address 0, the APICs, the 8259s and the 8254 of KVM, a CMOS clock, the PCI
      configuration ports with no device on the bus, and ACPI tables for --cpus processors.
      The kernel gets *nocga=1 and --bootargs as its multiboot command line and the --ini file
      as its first module. Its console is the 9h hypercall console with IRQ 5.
      --machine virt is for arm64 only. The machine needs /dev/kvm.";

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

/// The clipboard of `#Z/snarf`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Snarf {
    /// The clipboard of the host.
    Host,
    /// A buffer in the monitor.
    Memory,
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
    snarf: Snarf,
    strict: bool,
    timeout: Option<u64>,
    script: Option<Vec<(usize, Step)>>,
    headless: bool,
    /// The size of the screen, or None for no screen.
    screen: Option<(u32, u32)>,
    snap: Option<String>,
    control: Option<String>,
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
        snarf: Snarf::Host,
        strict: false,
        timeout: None,
        script: None,
        headless: false,
        screen: Some((1024, 768)),
        snap: None,
        control: None,
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
            "--snarf" => {
                o.snarf = match val()?.as_str() {
                    "host" => Snarf::Host,
                    "memory" => Snarf::Memory,
                    s => return Err(format!("--snarf {s}: the values are host and memory")),
                };
            }
            "--strict" => o.strict = true,
            "--headless" => o.headless = true,
            "--screen" => o.screen = screen_size(&val()?)?,
            "--snap" => o.snap = Some(val()?),
            "--control" => o.control = Some(val()?),
            "--script" => {
                let f = val()?;
                let src = std::fs::read_to_string(&f).map_err(|e| format!("{f}: {e}"))?;
                o.script = Some(script::parse(&src).map_err(|e| format!("{f}: {e}"))?);
            }
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
    check(&o)?;
    Ok(o)
}

/// The checks of the options together.
fn check(o: &Options) -> Result<(), String> {
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
    if o.machine == Machine::Virt && (!o.disks.is_empty() || o.hostfs.is_some() || o.snap.is_some())
    {
        return Err("--disk, --hostfs and --snap are for the 9h machine, not for virt".into());
    }
    if o.screen.is_none() && o.snap.is_some() {
        return Err("--snap needs a screen, not --screen none".into());
    }
    if let Some((d, _)) = &o.hostfs
        && !std::path::Path::new(d).is_dir()
    {
        return Err(format!("--hostfs {d}: not a directory"));
    }
    Ok(())
}

/// The size of `--screen WxH`, or None for `--screen none`.
fn screen_size(v: &str) -> Result<Option<(u32, u32)>, String> {
    if v == "none" {
        return Ok(None);
    }
    let bad = || {
        format!(
            "--screen {v}: give WxH, each 16 to {}, or none",
            h9screen::MAX
        )
    };
    let (w, h) = v.split_once('x').ok_or_else(bad)?;
    let (w, h): (u32, u32) = (w.parse().map_err(|_| bad())?, h.parse().map_err(|_| bad())?);
    if !(16..=h9screen::MAX).contains(&w) || !(16..=h9screen::MAX).contains(&h) {
        return Err(bad());
    }
    Ok(Some((w, h)))
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

/// The host runs the amd64 machine, not the arm64 one.
const PC: bool = cfg!(target_arch = "x86_64");

/// The RAM of the arm64 machine, with the kernel at its addresses, the device tree and the
/// plan9.ini, and the entry and the X0 of vCPU 0.
fn load_arm64(o: &Options, bytes: &[u8]) -> Result<(Arc<Ram>, u64, u64), String> {
    let image =
        h9_vm::kernel::load(bytes, KZERO, 0x1_0000).map_err(|e| format!("{}: {e}", o.kernel))?;
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
    Ok((ram, image.entry, DTB))
}

/// The variables that the amd64 machine gives the kernel before `--bootargs`. The machine has no
/// CGA, and `screeninit` of `pc/cga.c` reads the cursor of the CGA through ports 0x3d4 and 0x3d5
/// unless `*nocga` is set.
const PC_CMDLINE: &str = "*nocga=1";

/// The multiboot command line of the amd64 machine: `PC_CMDLINE`, then `--bootargs`. The kernel
/// takes each word as a line of plan9.ini.
fn pc_cmdline(bootargs: &str) -> String {
    if bootargs.is_empty() {
        PC_CMDLINE.into()
    } else {
        format!("{PC_CMDLINE} {bootargs}")
    }
}

/// The RAM of the amd64 machine, with the kernel at its addresses, the multiboot information
/// block and the ACPI tables, and the entry and the EBX of vCPU 0.
fn load_pc(o: &Options, bytes: &[u8]) -> Result<(Arc<Ram>, u64, u64), String> {
    let image = multiboot::load(bytes).map_err(|e| format!("{}: {e}", o.kernel))?;
    let mem = o.mem << 20;
    if image.end > mem {
        return Err(format!("{}: the kernel does not fit in the RAM", o.kernel));
    }
    let ram = Arc::new(Ram::new(0, mem as usize)?);
    for (pa, seg) in &image.segments {
        if !ram.write(*pa, seg) {
            return Err(format!(
                "{}: a segment at {pa:#x} is not in the RAM",
                o.kernel
            ));
        }
    }
    let ini = match &o.ini {
        Some(f) => Some(std::fs::read(f).map_err(|e| format!("{f}: {e}"))?),
        None => None,
    };
    let cmdline = pc_cmdline(&o.bootargs);
    let info = multiboot::info(&multiboot::Info {
        mem,
        cmdline: &cmdline,
        ini: ini.as_deref(),
    })?;
    if !ram.write(multiboot::INFO, &info) || !ram.write(acpi::RSDP, &acpi::tables(o.cpus)?) {
        return Err("the boot information does not fit in the RAM".into());
    }
    Ok((ram, image.entry, multiboot::INFO))
}

#[allow(clippy::too_many_lines)]
fn run(o: &Options) -> Result<ExitCode, String> {
    if PC && o.machine == Machine::Virt {
        return Err(
            "--machine virt is the arm64 bring-up machine, and this host runs amd64".into(),
        );
    }
    let bytes = std::fs::read(&o.kernel).map_err(|e| format!("{}: {e}", o.kernel))?;
    let (ram, entry, x0) = if PC {
        load_pc(o, &bytes)?
    } else {
        load_arm64(o, &bytes)?
    };
    let mut bus = Bus::new();
    bus.strict = o.strict;
    let log = o.script.as_ref().map(|_| Transcript::default());
    let console = || -> Box<dyn std::io::Write + Send> {
        match &log {
            Some(log) => Box::new(Tee {
                out: std::io::stdout(),
                log: log.clone(),
            }),
            None => Box::new(std::io::stdout()),
        }
    };
    match o.machine {
        Machine::Virt => bus.add(UART, 0x1000, Box::new(Pl011::new(UART_SPI, console()))),
        Machine::H9 => {
            let irq = if PC { h9::CONS_IRQ } else { h9::CONS_SPI };
            bus.add_hyper(Box::new(H9Cons::new(irq, console())));
        }
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
    if o.machine == Machine::H9 {
        let clip: Box<dyn Clipboard> = match (o.snarf, Host::find()) {
            (Snarf::Host, Some(h)) => Box::new(h),
            (Snarf::Host, None) => {
                eprintln!(
                    "9h: the host has no clipboard program, so the snarf stays in the monitor"
                );
                Box::new(Memory::default())
            }
            (Snarf::Memory, _) => Box::new(Memory::default()),
        };
        bus.add_hyper(Box::new(H9Snarf::new(clip)));
        let term: Option<Box<dyn std::io::Write + Send>> = std::io::stdout()
            .is_terminal()
            .then(|| Box::new(std::io::stdout()) as Box<dyn std::io::Write + Send>);
        bus.add_hyper(Box::new(H9Ctl::new(term)));
        // A screen of 0x0 tells the kernel that there is no screen.
        let (w, h) = o.screen.unwrap_or((0, 0));
        bus.add_hyper(Box::new(H9Screen::new(w, h)));
        let irq = if PC { h9::MOUSE_IRQ } else { h9::MOUSE_SPI };
        bus.add_hyper(Box::new(H9Mouse::new(irq)));
    }
    if PC {
        bus.add(PORTS + cmos::PORT, 2, Box::new(Cmos::new()));
        bus.add(
            PORTS + pcinone::PORT,
            pcinone::LEN,
            Box::new(PciNone::new()),
        );
        bus.add(PORTS + acpipm::PORT, acpipm::LEN, Box::new(AcpiPm::new()));
    } else {
        bus.add(RTC, 0x1000, Box::new(Pl031::new()));
    }
    let script = o.script.clone().zip(log);
    let control = match &o.control {
        Some(p) => Some(Control::bind(p)?),
        None => None,
    };
    if let (false, Machine::H9, Some((w, h)), true) =
        (o.headless, o.machine, o.screen, h9_gui::available())
    {
        // AppKit runs on the main thread, so the machine runs on a thread of its own. The X11
        // back end does the same.
        let window = Arc::new(backend::Window::default());
        let session = Session {
            script,
            snap: o.snap.clone(),
            control,
            window: Some(window.clone()),
        };
        let ncpu = o.cpus as usize;
        let done = window.clone();
        let t = std::thread::spawn(move || {
            let r = backend::run(ram, bus, ncpu, entry, x0, session);
            done.end();
            r
        });
        let title = format!("9h: {}", o.kernel);
        if let Err(e) = h9_gui::run(&title, w, h, window) {
            eprintln!("9h: the window: {e}");
        }
        return t
            .join()
            .unwrap_or_else(|_| Err("the thread of the machine panicked".into()));
    }
    let session = Session {
        script,
        snap: o.snap.clone(),
        control,
        window: None,
    };
    backend::run(ram, bus, o.cpus as usize, entry, x0, session)
}

/// The lines of the report of the addresses with no device that the guest used, one line for
/// each address with its count. A port of the amd64 machine shows as a port.
#[cfg_attr(
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )),
    allow(dead_code)
)]
fn unknown_report(bus: &Bus) -> Vec<String> {
    bus.unknown_accesses()
        .iter()
        .map(|(&pa, &n)| {
            if pa >= PORTS {
                format!("9h: no device: port {:#x}, {n} accesses", pa - PORTS)
            } else {
                format!("9h: no device: address {pa:#x}, {n} accesses")
            }
        })
        .collect()
}

/// Type the bytes on the console of the machine.
#[cfg_attr(
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )),
    allow(dead_code)
)]
fn type_in(bus: &mut Bus, b: &[u8]) {
    if let Some(u) = bus.device::<Pl011>(UART) {
        for &c in b {
            u.push(c);
        }
    } else if let Some(c) = bus.hyper_device::<H9Cons>() {
        for &x in b {
            c.push(x);
        }
    }
}

/// The number of typed bytes that the guest did not read yet.
#[cfg_attr(
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )),
    allow(dead_code)
)]
fn keys_pending(bus: &mut Bus) -> usize {
    if let Some(u) = bus.device::<Pl011>(UART) {
        u.pending()
    } else {
        bus.hyper_device::<H9Cons>().map_or(0, |c| c.pending())
    }
}

/// Give the input of a script step to the machine.
#[cfg_attr(
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )),
    allow(dead_code)
)]
fn give(bus: &mut Bus, i: &script::Input) {
    match *i {
        script::Input::Keys(b) => type_in(bus, b),
        script::Input::Mouse(x, y, buttons) => {
            if let Some(m) = bus.hyper_device::<H9Mouse>() {
                m.push(h9mouse::Event { x, y, buttons });
            }
        }
    }
}

/// Write the screen to the PNG file `f` at the end of a session, and give the line for the log.
#[cfg_attr(
    not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(
            target_os = "linux",
            any(target_arch = "x86_64", target_arch = "aarch64")
        )
    )),
    allow(dead_code)
)]
fn snap(bus: &mut Bus, f: &str) -> Result<String, String> {
    let s = bus
        .hyper_device::<H9Screen>()
        .ok_or("the machine has no screen")?;
    if !s.on() {
        return Err("the kernel did not give a framebuffer".into());
    }
    std::fs::write(f, s.png()).map_err(|e| e.to_string())?;
    let (w, h) = s.size();
    Ok(format!(
        "9h: the screen ({w}x{h}, {} colours, {} flushes) is in {f}",
        s.colours(),
        s.flushes
    ))
}

/// The socket of `--control PATH`, on which a program of the host drives the machine: the steps
/// `send`, `mouse`, `click`, `pause`, `quiet` and `snap`, one line each. The monitor answers each line with
/// `ok` or `error: ` and the reason, after it did the steps. `r9 9h boot --snap` uses it to click
/// in the console window of rio. The socket file goes away at the end of the session.
struct Control {
    listener: std::os::unix::net::UnixListener,
    path: String,
}

impl Control {
    fn bind(path: &str) -> Result<Control, String> {
        let listener = std::os::unix::net::UnixListener::bind(path)
            .map_err(|e| format!("--control {path}: {e}"))?;
        Ok(Control {
            listener,
            path: path.into(),
        })
    }

    /// Answer each connection in a thread of its own.
    #[cfg_attr(
        not(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(
                target_os = "linux",
                any(target_arch = "x86_64", target_arch = "aarch64")
            )
        )),
        allow(dead_code)
    )]
    fn serve(&self, target: impl script::Target + Clone + Send + 'static) {
        use std::io::{BufRead, Write};
        let Ok(listener) = self.listener.try_clone() else {
            eprintln!("9h: --control {}: the socket cannot be shared", self.path);
            return;
        };
        std::thread::spawn(move || {
            for c in listener.incoming().flatten() {
                let mut target = target.clone();
                std::thread::spawn(move || {
                    let Ok(mut out) = c.try_clone() else { return };
                    for line in std::io::BufReader::new(c).lines().map_while(Result::ok) {
                        let reply = script::control(&line, &mut target);
                        if writeln!(out, "{reply}").is_err() {
                            break;
                        }
                    }
                });
            }
        });
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A session script and the console output that it reads.
type Script = (Vec<(usize, Step)>, Transcript);

/// The parts of a session other than the machine.
struct Session {
    /// The script, which owns the input, or None for the standard input.
    script: Option<Script>,
    /// The PNG file for the screen at the end.
    snap: Option<String>,
    /// The socket of `--control`.
    control: Option<Control>,
    /// The host window, which gets the machine when the machine is made.
    window: Option<Arc<backend::Window>>,
}

/// The back end of the host: HVF on macOS on arm64, KVM on Linux on x86-64 and arm64. The two
/// have the same interface.
#[cfg(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
))]
mod backend {
    use super::{H9Screen, Session, give, keys_pending, script, type_in, unknown_report};
    use h9_dev::Bus;
    #[cfg(target_os = "macos")]
    use h9_hvf::{Machine, Stop, Vm};
    #[cfg(target_os = "linux")]
    use h9_kvm::{Machine, Stop, Vm};
    use h9_vm::Ram;
    use std::io::Read;
    use std::process::ExitCode;
    use std::sync::atomic::{AtomicI32, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    fn off_screen(s: &H9Screen) -> String {
        let (w, h) = s.size();
        format!("the rectangle is not all on the screen of {w}x{h}")
    }

    /// The machine as the target of the steps of a script or of the control socket.
    #[derive(Clone)]
    struct Drive(Arc<Machine>);

    impl script::Target for Drive {
        /// Keys go to the guest a few at a time, as a person types them. A long line in one
        /// burst loses bytes in the guest, after the console device: in a test, `rio` got the
        /// first 150 bytes of a line of 210 bytes and the newline, and not the bytes between.
        fn give(&mut self, i: script::Input) {
            const CHUNK: usize = 8;
            const GAP: Duration = Duration::from_millis(20);
            let script::Input::Keys(keys) = i else {
                self.0.with_bus(|bus| give(bus, &i));
                return;
            };
            for chunk in keys.chunks(CHUNK) {
                self.0.with_bus(|bus| type_in(bus, chunk));
                let limit = Instant::now() + Duration::from_secs(2);
                while self.0.with_bus(keys_pending) > 0 && Instant::now() < limit {
                    std::thread::sleep(Duration::from_millis(2));
                }
                std::thread::sleep(GAP);
            }
        }

        fn look(&mut self, rect: Option<[u16; 4]>) -> Result<u64, String> {
            self.0.with_bus(|bus| {
                let s = bus
                    .hyper_device::<H9Screen>()
                    .ok_or("the machine has no screen")?;
                if !s.on() {
                    return Err("the kernel did not give a framebuffer".to_string());
                }
                match rect {
                    None => Ok(s.generation()),
                    Some([x0, y0, x1, y1]) => s
                        .digest(x0.into(), y0.into(), x1.into(), y1.into())
                        .ok_or_else(|| off_screen(s)),
                }
            })
        }

        fn snap(&mut self, file: &str, rect: Option<[u16; 4]>) -> Result<(), String> {
            let png = self.0.with_bus(|bus| {
                let s = bus
                    .hyper_device::<H9Screen>()
                    .ok_or("the machine has no screen")?;
                if !s.on() {
                    return Err("the kernel did not give a framebuffer".to_string());
                }
                match rect {
                    None => Ok(s.png()),
                    Some([x0, y0, x1, y1]) => s
                        .png_rect(x0.into(), y0.into(), x1.into(), y1.into())
                        .ok_or_else(|| off_screen(s)),
                }
            })?;
            std::fs::write(file, png).map_err(|e| e.to_string())
        }
    }

    /// The host of the window (`h9-gui`): the screen, the keyboard and the mouse of the machine.
    #[derive(Default)]
    pub struct Window {
        pub machine: std::sync::OnceLock<Arc<Machine>>,
        done: std::sync::atomic::AtomicBool,
    }

    impl Window {
        /// The session ended: the window closes.
        pub fn end(&self) {
            self.done.store(true, Ordering::SeqCst);
        }
    }

    impl h9_gui::Host for Window {
        fn frame(&self, generation: &mut u64, pixels: &mut Vec<u32>) -> Option<(u32, u32)> {
            self.machine.get()?.with_bus(|bus| {
                let s = bus.hyper_device::<h9_dev::h9screen::H9Screen>()?;
                if !s.on() || s.generation() == *generation {
                    return None;
                }
                *generation = s.generation();
                pixels.clear();
                pixels.extend_from_slice(s.pixels());
                Some(s.size())
            })
        }

        fn mouse(&self, x: u16, y: u16, buttons: u8) {
            if let Some(m) = self.machine.get() {
                m.with_bus(|bus| give(bus, &super::script::Input::Mouse(x, y, buttons)));
            }
        }

        fn keys(&self, utf8: &[u8]) {
            if let Some(m) = self.machine.get() {
                m.with_bus(|bus| type_in(bus, utf8));
            }
        }

        fn quit(&self) {
            if let Some(m) = self.machine.get() {
                m.end(Stop::Off);
            }
        }

        fn done(&self) -> bool {
            self.done.load(Ordering::SeqCst)
        }
    }

    /// The number of the first SIGTERM or SIGINT that came, or 0.
    static SIGNAL: AtomicI32 = AtomicI32::new(0);

    extern "C" fn caught(sig: libc::c_int) {
        if SIGNAL.swap(sig, Ordering::SeqCst) != 0 {
            // The second signal: the session did not end after the first, so stop now.
            // SAFETY: _exit is async-signal-safe and takes no pointer.
            unsafe { libc::_exit(128 + sig) };
        }
    }

    /// End the session as the guest does with `SYSTEM_OFF` when the monitor gets SIGTERM or
    /// SIGINT. Thus the monitor writes the screen and the report of unknown accesses before it
    /// exits. `r9 9h boot` stops the monitor with SIGTERM after `fshalt`, because the arm64 kernel
    /// does not turn the machine off. A second signal stops the monitor at once.
    fn on_signal(m: Arc<Machine>) {
        let handler = caught as extern "C" fn(libc::c_int) as libc::sighandler_t;
        // SAFETY: `caught` only does atomic operations and `_exit`, which are async-signal-safe.
        unsafe {
            libc::signal(libc::SIGTERM, handler);
            libc::signal(libc::SIGINT, handler);
        }
        std::thread::spawn(move || {
            while SIGNAL.load(Ordering::SeqCst) == 0 {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            m.end(Stop::Off);
        });
    }

    pub fn run(
        ram: Arc<Ram>,
        bus: Bus,
        ncpu: usize,
        entry: u64,
        x0: u64,
        session: Session,
    ) -> Result<ExitCode, String> {
        let Session {
            script,
            snap,
            control,
            window,
        } = session;
        let vm = Vm::new(ram, bus, ncpu)?;
        let m = vm.machine();
        if let Some(w) = &window {
            let _ = w.machine.set(vm.machine());
        }
        if let Some(c) = &control {
            let m = vm.machine();
            c.serve(Drive(m));
        }
        let done = Arc::new(Mutex::new(None));
        let scripted = script.is_some();
        if let Some((steps, log)) = script {
            let done = done.clone();
            std::thread::spawn(move || {
                let r = script::run(&steps, &log, &mut Drive(m.clone()));
                *done.lock().unwrap() = Some(r);
                m.end(Stop::Off);
            });
        } else {
            std::thread::spawn(move || {
                let mut b = [0u8; 256];
                let mut stdin = std::io::stdin();
                while let Ok(n @ 1..) = stdin.read(&mut b) {
                    m.with_bus(|bus| type_in(bus, &b[..n]));
                }
            });
        }
        on_signal(vm.machine());
        let stop = vm.run(entry, x0);
        let signalled = SIGNAL.load(Ordering::SeqCst) != 0;
        for line in vm.machine().with_bus(|bus| unknown_report(bus)) {
            eprintln!("{line}");
        }
        if let Some(f) = snap {
            let r = vm.machine().with_bus(|bus| super::snap(bus, &f));
            match r {
                Ok(line) => eprintln!("{line}"),
                Err(e) => eprintln!("9h: --snap {f}: {e}"),
            }
        }
        let stop = stop?;
        if scripted {
            return Ok(match done.lock().unwrap().take() {
                Some(Ok(())) => {
                    eprintln!("\n9h: the script passed");
                    ExitCode::SUCCESS
                }
                Some(Err(e)) => {
                    eprintln!("\n9h: the script failed: {e}");
                    ExitCode::from(3)
                }
                None => {
                    eprintln!("\n9h: the session ended before the script did ({stop:?})");
                    ExitCode::from(3)
                }
            });
        }
        match stop {
            Stop::Off if signalled => eprintln!("9h: the session ended on a signal"),
            Stop::Off => eprintln!("9h: the guest turned the machine off"),
            Stop::Reset => eprintln!("9h: the guest asked for a reset"),
        }
        Ok(ExitCode::SUCCESS)
    }
}

#[cfg(not(any(
    all(target_os = "macos", target_arch = "aarch64"),
    all(
        target_os = "linux",
        any(target_arch = "x86_64", target_arch = "aarch64")
    )
)))]
mod backend {
    use h9_dev::Bus;
    use h9_vm::Ram;
    use std::process::ExitCode;
    use std::sync::Arc;

    /// These hosts have no window, and `h9_gui::available` is false.
    #[derive(Default)]
    pub struct Window;

    impl Window {
        pub const fn end(&self) {}
    }

    impl h9_gui::Host for Window {
        fn frame(&self, _: &mut u64, _: &mut Vec<u32>) -> Option<(u32, u32)> {
            None
        }
        fn mouse(&self, _: u16, _: u16, _: u8) {}
        fn keys(&self, _: &[u8]) {}
        fn quit(&self) {}
        fn done(&self) -> bool {
            true
        }
    }

    pub fn run(
        _: Arc<Ram>,
        _: Bus,
        _: usize,
        _: u64,
        _: u64,
        _: super::Session,
    ) -> Result<ExitCode, String> {
        Err("this host has no back end: the monitor runs on macOS on arm64 (HVF) and on Linux on x86-64 and arm64 (KVM)".into())
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
    use super::{Bus, PORTS, devtree, parse, pc_cmdline, unknown_report};

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
        assert_eq!(o.snarf, super::Snarf::Host);
        assert_eq!(o.user, "tam");
        let o = parse(&args("--kernel k --snarf memory")).unwrap();
        assert_eq!(o.snarf, super::Snarf::Memory);
        assert!(parse(&args("--kernel k --snarf x11")).is_err());
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
    fn the_screen_options_are_read() {
        let o = parse(&args("--kernel k")).unwrap();
        assert_eq!((o.screen, o.snap), (Some((1024, 768)), None));
        let o = parse(&args("--kernel k --screen 1920x1080 --snap s.png")).unwrap();
        assert_eq!(
            (o.screen, o.snap.as_deref()),
            (Some((1920, 1080)), Some("s.png"))
        );
        let o = parse(&args("--kernel k --screen none")).unwrap();
        assert_eq!(o.screen, None);
        assert!(parse(&args("--kernel k --screen none --snap s.png")).is_err());
        for bad in ["1024", "8x768", "1024x0", "1024x9000", "axb"] {
            assert!(
                parse(&args(&format!("--kernel k --screen {bad}"))).is_err(),
                "{bad}"
            );
        }
        assert!(parse(&args("--kernel k --machine virt --snap s.png")).is_err());
    }

    #[test]
    fn the_pc_kernel_gets_nocga_before_the_bootargs() {
        assert_eq!(pc_cmdline(""), "*nocga=1");
        assert_eq!(pc_cmdline("console=0"), "*nocga=1 console=0");
    }

    #[test]
    fn the_report_names_each_address_with_no_device() {
        let mut bus = Bus::new();
        bus.write(PORTS + 0x3d4, 1, 0xe).unwrap();
        bus.read(PORTS + 0x3d5, 1).unwrap();
        bus.read(PORTS + 0x3d5, 1).unwrap();
        bus.read(0xfec0_0000, 4).unwrap();
        assert_eq!(
            unknown_report(&bus),
            [
                "9h: no device: address 0xfec00000, 1 accesses",
                "9h: no device: port 0x3d4, 1 accesses",
                "9h: no device: port 0x3d5, 2 accesses",
            ]
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
