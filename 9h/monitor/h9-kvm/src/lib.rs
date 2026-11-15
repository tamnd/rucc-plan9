//! The KVM back end of the 9h monitor, for amd64 on Linux (plan 10.4).
//!
//! It makes the PC that `9pc64` of the tree knows. KVM gives the local APICs, the I/O APIC and the
//! two 8259s (`KVM_CREATE_IRQCHIP`), and the 8254 timer (`KVM_CREATE_PIT2`). The monitor gives the
//! RAM from address 0, the devices of the bus, and the 9h hypercalls. A hypercall is an `OUT` to
//! `HYPERCALL_PORT`, with the number in RAX and the arguments in RBX, RCX, RSI, RDI and R8. The
//! result goes in RAX. An access to another port goes to the bus at `PORTS` plus the port.
//!
//! A write with bit 2 set to `RESET_PORT`, the reset control register of the PC, resets the
//! machine. `archreset` of the tree writes 6 there when the kernel exits or reboots.
//!
//! vCPU 0 starts at the multiboot entry of the kernel, in 32-bit protected mode with no paging,
//! with `EAX` and `EBX` as the multiboot specification says. The other vCPUs wait in KVM until
//! the kernel starts them with INIT and SIPI through its local APIC. Each vCPU runs on a thread of
//! its own.
//!
//! The monitor needs KVM. It has no emulator to use when the host has no KVM (open question 13).
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use h9_dev::psci::{self, Action};
use h9_dev::{Bus, PORTS};
use h9_vm::Ram;
use h9_vm::multiboot;
use kvm_bindings::{
    KVM_MAX_CPUID_ENTRIES, KVM_PIT_SPEAKER_DUMMY, KVM_SYSTEM_EVENT_RESET,
    KVM_SYSTEM_EVENT_SHUTDOWN, kvm_pit_config, kvm_regs, kvm_segment, kvm_userspace_memory_region,
};
use kvm_ioctls::{Kvm, VcpuExit, VcpuFd, VmFd};

/// The port of the 9h hypercalls.
pub const HYPERCALL_PORT: u16 = 0x509;
/// The reset control register of the PC.
pub const RESET_PORT: u16 = 0xcf9;
/// The bit of `RESET_PORT` that starts the reset.
const RESET_CPU: u8 = 1 << 2;
/// The three pages that KVM needs for the TSS of a real-mode guest on Intel, below the BIOS.
const TSS: usize = 0xfffb_d000;
/// The signal that makes a vCPU thread leave `KVM_RUN`.
const KICK: libc::c_int = libc::SIGUSR1;

/// Why the session ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// The guest called `SYSTEM_OFF`, or a device asked to end the session.
    Off,
    /// The guest called `SYSTEM_RESET`, or made a triple fault.
    Reset,
}

/// The state that the vCPU threads and the device threads share: the VM, the bus, the levels of
/// the interrupt lines that KVM has now, the threads that run a vCPU, and the end of the session.
pub struct Machine {
    /// The RAM of the guest.
    pub ram: Arc<Ram>,
    vm: VmFd,
    bus: Mutex<Bus>,
    levels: Mutex<Vec<(u32, bool)>>,
    stopping: AtomicBool,
    /// The thread of each vCPU while it runs. A thread clears its slot before it ends.
    threads: Mutex<Vec<Option<libc::pthread_t>>>,
    end: (Mutex<Option<Result<Stop, String>>>, Condvar),
}

/// The virtual machine, with its vCPUs before the session starts.
pub struct Vm {
    machine: Arc<Machine>,
    vcpus: Mutex<Vec<VcpuFd>>,
}

fn kvm_error(what: &str) -> impl Fn(kvm_ioctls::Error) -> String + '_ {
    move |e| format!("{what}: {e}")
}

/// The answer when `/dev/kvm` does not open.
fn no_kvm(e: kvm_ioctls::Error) -> String {
    format!(
        "cannot open /dev/kvm: {e}. The amd64 machine of 9h needs KVM: the kvm module must be loaded, and the user must be able to read and write /dev/kvm, as a member of its group. The monitor has no emulator for a host with no KVM."
    )
}

impl Vm {
    /// Make the VM with `ncpu` processors and its devices in KVM, and map the RAM, which must
    /// start at address 0.
    pub fn new(ram: Arc<Ram>, bus: Bus, ncpu: usize) -> Result<Vm, String> {
        if ram.base() != 0 {
            return Err("the RAM of the amd64 machine starts at address 0".into());
        }
        let kvm = Kvm::new().map_err(no_kvm)?;
        let vm = kvm.create_vm().map_err(kvm_error("KVM_CREATE_VM"))?;
        vm.set_tss_address(TSS)
            .map_err(kvm_error("KVM_SET_TSS_ADDR"))?;
        vm.create_irq_chip()
            .map_err(kvm_error("KVM_CREATE_IRQCHIP"))?;
        vm.create_pit2(kvm_pit_config {
            flags: KVM_PIT_SPEAKER_DUMMY,
            ..Default::default()
        })
        .map_err(kvm_error("KVM_CREATE_PIT2"))?;
        let region = kvm_userspace_memory_region {
            slot: 0,
            flags: 0,
            guest_phys_addr: 0,
            memory_size: ram.len() as u64,
            userspace_addr: ram.host_ptr() as u64,
        };
        // SAFETY: the RAM lives as long as the Machine, which owns the VM, so the mapping stays
        // valid while the guest can use it.
        unsafe { vm.set_user_memory_region(region) }
            .map_err(kvm_error("KVM_SET_USER_MEMORY_REGION"))?;
        let cpuid = kvm
            .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
            .map_err(kvm_error("KVM_GET_SUPPORTED_CPUID"))?;
        let mut vcpus = Vec::new();
        for n in 0..ncpu.max(1) {
            let cpu = vm
                .create_vcpu(n as u64)
                .map_err(kvm_error("KVM_CREATE_VCPU"))?;
            // The kernel finds the APIC ID of each processor in CPUID, and it must be the one of
            // the MADT, which is the number of the processor.
            let mut id = cpuid.clone();
            for e in id.as_mut_slice() {
                match e.function {
                    1 => e.ebx = e.ebx & 0x00ff_ffff | (n as u32) << 24,
                    0xb | 0x1f => e.edx = n as u32,
                    _ => {}
                }
            }
            cpu.set_cpuid2(&id).map_err(kvm_error("KVM_SET_CPUID2"))?;
            vcpus.push(cpu);
        }
        let machine = Machine {
            ram,
            vm,
            bus: Mutex::new(bus),
            levels: Mutex::new(Vec::new()),
            stopping: AtomicBool::new(false),
            threads: Mutex::new(vec![None; vcpus.len()]),
            end: (Mutex::new(None), Condvar::new()),
        };
        Ok(Vm {
            machine: Arc::new(machine),
            vcpus: Mutex::new(vcpus),
        })
    }

    /// The state that a device thread needs.
    #[must_use]
    pub fn machine(&self) -> Arc<Machine> {
        self.machine.clone()
    }

    /// Start vCPU 0 at the multiboot `entry` with `info` in EBX, start a thread for each vCPU,
    /// and wait until the session ends.
    pub fn run(&self, entry: u64, info: u64) -> Result<Stop, String> {
        let vcpus = std::mem::take(&mut *self.vcpus.lock().unwrap());
        let Some(first) = vcpus.first() else {
            return Err("the session was run already".into());
        };
        boot(first, entry, info)?;
        install_kick()?;
        let m = &self.machine;
        for (n, cpu) in vcpus.into_iter().enumerate() {
            let m = m.clone();
            // Register the thread before it can run, so that the kick at the end finds it.
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                // SAFETY: pthread_self has no preconditions.
                m.threads.lock().unwrap()[n] = Some(unsafe { libc::pthread_self() });
                let _ = tx.send(());
                let mut cpu = cpu;
                let r = run_vcpu(&m, &mut cpu);
                m.threads.lock().unwrap()[n] = None;
                match r {
                    Ok(Some(stop)) => m.finish(Ok(stop)),
                    Ok(None) => {}
                    Err(e) => m.finish(Err(format!("cpu{n}: {e}"))),
                }
            });
            let _ = rx.recv();
        }
        let mut end = m.end.0.lock().unwrap();
        while end.is_none() {
            end = m.end.1.wait(end).unwrap();
        }
        let r = end.take().unwrap();
        drop(end);
        // Kick each vCPU out of KVM_RUN until each thread has ended. A kick can come just before
        // a thread enters KVM_RUN, so the kicks go on until the threads are gone.
        loop {
            let threads = m.threads.lock().unwrap();
            if threads.iter().all(Option::is_none) {
                break;
            }
            for t in threads.iter().flatten() {
                // SAFETY: the thread is alive, because it clears its slot under this lock before
                // it ends. The signal has a handler that does nothing.
                unsafe { libc::pthread_kill(*t, KICK) };
            }
            drop(threads);
            std::thread::sleep(Duration::from_millis(1));
        }
        r
    }
}

extern "C" fn kicked(_: libc::c_int) {}

/// Give `KICK` a handler that does nothing, with no `SA_RESTART`, so that the signal only makes
/// `KVM_RUN` return `EINTR`.
fn install_kick() -> Result<(), String> {
    // SAFETY: the sigaction is zeroed, then filled; the handler is a function that does nothing,
    // which is safe in a signal.
    let r = unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = kicked as extern "C" fn(libc::c_int) as libc::sighandler_t;
        libc::sigemptyset(&raw mut sa.sa_mask);
        libc::sigaction(KICK, &raw const sa, std::ptr::null_mut())
    };
    if r != 0 {
        return Err(format!("sigaction: {}", std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Put vCPU 0 in the state that the multiboot specification gives at the entry: 32-bit protected
/// mode with flat segments, no paging, no interrupts, and the magic number in EAX.
fn boot(cpu: &VcpuFd, entry: u64, info: u64) -> Result<(), String> {
    let mut s = cpu.get_sregs().map_err(kvm_error("KVM_GET_SREGS"))?;
    let code = kvm_segment {
        base: 0,
        limit: 0xffff_ffff,
        selector: 0x08,
        type_: 0xb,
        present: 1,
        dpl: 0,
        db: 1,
        s: 1,
        l: 0,
        g: 1,
        avl: 0,
        unusable: 0,
        padding: 0,
    };
    let data = kvm_segment {
        selector: 0x10,
        type_: 0x3,
        ..code
    };
    s.cs = code;
    s.ds = data;
    s.es = data;
    s.fs = data;
    s.gs = data;
    s.ss = data;
    // PE and ET. The caches are on, and the kernel turns on paging itself.
    s.cr0 = 0x11;
    s.cr4 = 0;
    s.efer = 0;
    cpu.set_sregs(&s).map_err(kvm_error("KVM_SET_SREGS"))?;
    let regs = kvm_regs {
        rip: entry,
        rax: u64::from(multiboot::BOOT_MAGIC),
        rbx: info,
        rflags: 2,
        ..Default::default()
    };
    cpu.set_regs(&regs).map_err(kvm_error("KVM_SET_REGS"))
}

/// The little-endian value of the bytes of an access.
fn value(data: &[u8]) -> u64 {
    data.iter().rev().fold(0, |v, &b| v << 8 | u64::from(b))
}

fn fill(data: &mut [u8], v: u64) {
    for (n, b) in data.iter_mut().enumerate() {
        *b = (v >> (8 * n)) as u8;
    }
}

/// The accesses of a port exit. A string instruction (`INS`, `OUTS`) gives the data of several
/// accesses. The size of each is not in the slice, so such data is taken as accesses of one byte,
/// which the 9h kernel does not make to a device of the monitor.
fn chunks(len: usize) -> usize {
    if matches!(len, 1 | 2 | 4) { len } else { 1 }
}

/// What the loop does after an exit.
enum Next {
    Run,
    Hypercall,
    End(Stop),
}

/// Run the vCPU until the session ends, which gives `Some`, or until another vCPU ends it.
fn run_vcpu(m: &Machine, cpu: &mut VcpuFd) -> Result<Option<Stop>, String> {
    loop {
        if m.stopping.load(Ordering::SeqCst) {
            return Ok(None);
        }
        let next = match cpu.run() {
            Ok(VcpuExit::IoOut(HYPERCALL_PORT, _)) => Next::Hypercall,
            Ok(VcpuExit::IoOut(RESET_PORT, [v])) if v & RESET_CPU != 0 => Next::End(Stop::Reset),
            Ok(VcpuExit::IoOut(port, data)) => {
                let size = chunks(data.len());
                for c in data.chunks(size) {
                    m.with_bus(|b| b.write(PORTS + u64::from(port), size as u8, value(c)))?;
                }
                Next::Run
            }
            Ok(VcpuExit::IoIn(port, data)) => {
                let size = chunks(data.len());
                for c in data.chunks_mut(size) {
                    let v = m.with_bus(|b| b.read(PORTS + u64::from(port), size as u8))?;
                    fill(c, v);
                }
                Next::Run
            }
            Ok(VcpuExit::MmioRead(pa, data)) => {
                let v = m.with_bus(|b| b.read(pa, data.len() as u8))?;
                fill(data, v);
                Next::Run
            }
            Ok(VcpuExit::MmioWrite(pa, data)) => {
                m.with_bus(|b| b.write(pa, data.len() as u8, value(data)))?;
                Next::Run
            }
            // With the interrupt controllers in KVM, HLT waits in KVM, and a signal gives EINTR.
            Ok(VcpuExit::Hlt | VcpuExit::Intr) => Next::Run,
            Ok(VcpuExit::Shutdown) => {
                eprintln!("9h: the guest made a triple fault, and the PC resets");
                Next::End(Stop::Reset)
            }
            Ok(VcpuExit::SystemEvent(KVM_SYSTEM_EVENT_SHUTDOWN, _)) => Next::End(Stop::Off),
            Ok(VcpuExit::SystemEvent(KVM_SYSTEM_EVENT_RESET, _)) => Next::End(Stop::Reset),
            Ok(VcpuExit::FailEntry(reason, _)) => {
                return Err(format!(
                    "KVM could not enter the vCPU (hardware entry failure reason {reason:#x})"
                ));
            }
            Ok(e) => return Err(format!("the vCPU stopped with the exit {e:?}")),
            Err(e) if e.errno() == libc::EINTR || e.errno() == libc::EAGAIN => Next::Run,
            Err(e) => return Err(format!("KVM_RUN: {e}")),
        };
        match next {
            Next::Run => {}
            Next::End(stop) => return Ok(Some(stop)),
            Next::Hypercall => {
                if let Some(stop) = hypercall(m, cpu)? {
                    return Ok(Some(stop));
                }
            }
        }
    }
}

/// Do the hypercall of the vCPU, and put the result in RAX. KVM moves RIP past the `OUT` when the
/// vCPU runs again.
fn hypercall(m: &Machine, cpu: &VcpuFd) -> Result<Option<Stop>, String> {
    let mut r = cpu.get_regs().map_err(kvm_error("KVM_GET_REGS"))?;
    let x = [r.rax, r.rbx, r.rcx, r.rsi, r.rdi, r.r8];
    r.rax = match psci::call(&x) {
        Some(Action::Return(v)) => v,
        Some(Action::Off) => return Ok(Some(Stop::Off)),
        Some(Action::Reset) => return Ok(Some(Stop::Reset)),
        // The PC starts and stops its processors through the local APICs.
        Some(_) => psci::NOT_SUPPORTED,
        None => {
            let (v, stop) = m.with_bus(|b| (b.call(&x, &m.ram), b.stopping()));
            if stop {
                return Ok(Some(Stop::Off));
            }
            v
        }
    };
    cpu.set_regs(&r).map_err(kvm_error("KVM_SET_REGS"))?;
    Ok(None)
}

impl Machine {
    /// Run `f` on the bus, then give KVM each interrupt line whose level changed.
    pub fn with_bus<T>(&self, f: impl FnOnce(&mut Bus) -> T) -> T {
        let mut bus = self.bus.lock().unwrap();
        let t = f(&mut bus);
        let now = bus.irqs();
        drop(bus);
        let mut levels = self.levels.lock().unwrap();
        for (irq, level) in now {
            let changed = match levels.iter_mut().find(|(i, _)| *i == irq) {
                Some(o) if o.1 == level => false,
                Some(o) => {
                    o.1 = level;
                    true
                }
                None => {
                    levels.push((irq, level));
                    level
                }
            };
            if changed && let Err(e) = self.vm.set_irq_line(irq, level) {
                eprintln!("9h: KVM_IRQ_LINE {irq}: {e}");
            }
        }
        t
    }

    /// End the session with `stop`, as the guest does with `SYSTEM_OFF`. A thread of the monitor
    /// calls this, such as the one of a session script.
    pub fn end(&self, stop: Stop) {
        self.finish(Ok(stop));
    }

    /// End the session with `r`, unless it has ended already. `Vm::run` then stops each vCPU.
    fn finish(&self, r: Result<Stop, String>) {
        self.stopping.store(true, Ordering::SeqCst);
        let mut end = self.end.0.lock().unwrap();
        if end.is_none() {
            *end = Some(r);
        }
        drop(end);
        self.end.1.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::{chunks, fill, value};

    #[test]
    fn the_data_of_an_access_is_little_endian() {
        assert_eq!(value(&[0x78, 0x56, 0x34, 0x12]), 0x1234_5678);
        let mut b = [0u8; 2];
        fill(&mut b, 0xffff_abcd);
        assert_eq!(b, [0xcd, 0xab]);
        assert_eq!(chunks(4), 4);
        assert_eq!(chunks(12), 1);
    }
}
