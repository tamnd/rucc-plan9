//! The Hypervisor.framework back end of the 9h monitor, for arm64 on macOS (plan 10.3).
//!
//! It makes the machine that the arm64 code of the tree knows: the layout of the QEMU `virt`
//! machine, with the `GICv3` of the framework and no PCI. It runs one vCPU on the thread that made
//! it, and it sends each exit to the devices of the bus.
#![cfg(all(target_os = "macos", target_arch = "aarch64"))]

pub mod ffi;

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use ffi::check;
use h9_dev::Bus;
use h9_dev::psci::{self, Action};
use h9_vm::{Exit, Ram};

/// The guest physical address of the `GICv3` distributor.
pub const GICD: u64 = 0x0800_0000;
/// The guest physical address of the first `GICv3` redistributor.
pub const GICR: u64 = 0x080a_0000;
/// The interrupt ID of the virtual timer of EL1, a private peripheral interrupt.
pub const VTIMER_INTID: u32 = 27;

/// Why the session ended.
#[derive(Debug, PartialEq, Eq)]
pub enum Stop {
    /// The guest called `SYSTEM_OFF`, or vCPU 0 called `CPU_OFF`.
    Off,
    /// The guest called `SYSTEM_RESET`.
    Reset,
}

/// The state that the vCPU threads and the device threads share: the bus, the levels of the
/// interrupt lines that the GIC has now, and the wake-up of a vCPU in `WFI`.
pub struct Machine {
    /// The RAM of the guest.
    pub ram: Arc<Ram>,
    bus: Mutex<Bus>,
    levels: Mutex<Vec<(u32, bool)>>,
    wake: (Mutex<bool>, Condvar),
}

/// The one virtual machine of the process. The framework allows only one.
pub struct Vm {
    machine: Arc<Machine>,
}

impl Vm {
    /// Make the VM, map the RAM, and make the GIC.
    pub fn new(ram: Arc<Ram>, bus: Bus) -> Result<Vm, String> {
        // SAFETY: a null configuration asks for the default VM.
        check("hv_vm_create", unsafe {
            ffi::hv_vm_create(std::ptr::null_mut())
        })?;
        let flags = ffi::HV_MEMORY_READ | ffi::HV_MEMORY_WRITE | ffi::HV_MEMORY_EXEC;
        // SAFETY: the RAM lives as long as the Machine, which outlives the VM, and its size is a
        // multiple of the page size of the host.
        check("hv_vm_map", unsafe {
            ffi::hv_vm_map(ram.host_ptr().cast(), ram.base(), ram.len(), flags)
        })?;
        gic()?;
        let machine = Machine {
            ram,
            bus: Mutex::new(bus),
            levels: Mutex::new(Vec::new()),
            wake: (Mutex::new(false), Condvar::new()),
        };
        Ok(Vm {
            machine: Arc::new(machine),
        })
    }

    /// The state that a device thread needs.
    #[must_use]
    pub fn machine(&self) -> Arc<Machine> {
        self.machine.clone()
    }

    /// Run vCPU 0 from `entry` with `x0` in X0, on this thread, until the session ends.
    pub fn run(&self, entry: u64, x0: u64) -> Result<Stop, String> {
        let mut cpu = Vcpu::new(0)?;
        if trace() {
            let id = cpu.id;
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(500));
                    // SAFETY: one vCPU id, which stays valid while the VM runs.
                    unsafe { ffi::hv_vcpus_exit(&raw const id, 1) };
                }
            });
        }
        cpu.set_reg(ffi::HV_REG_PC, entry)?;
        cpu.set_reg(ffi::HV_REG_X0, x0)?;
        // EL1 with SP_EL1, and the D, A, I and F exceptions masked, as a boot loader leaves it.
        cpu.set_reg(ffi::HV_REG_CPSR, 0x3c5)?;
        // The framework emulates the PMU only when PMUVer of ID_AA64DFR0_EL1 is not zero, and it
        // is zero by default. Without this, the PMCR_EL0 store of the clockinit of the tree takes
        // an Undefined exception in the guest, and the monitor does not see it.
        let dfr0 = cpu.sys(ffi::HV_SYS_REG_ID_AA64DFR0_EL1);
        if dfr0 & 0xf00 == 0 {
            cpu.set_sys(ffi::HV_SYS_REG_ID_AA64DFR0_EL1, dfr0 | 1 << 8)?;
        }
        cpu.run_loop(&self.machine)
    }
}

impl Drop for Vm {
    fn drop(&mut self) {
        // SAFETY: each vCPU was destroyed when its Vcpu was dropped.
        unsafe { ffi::hv_vm_destroy() };
    }
}

fn gic() -> Result<(), String> {
    let (mut da, mut ra, mut rsize) = (0usize, 0usize, 0usize);
    // SAFETY: each call writes one usize through a valid pointer.
    unsafe {
        check(
            "hv_gic_get_distributor_base_alignment",
            ffi::hv_gic_get_distributor_base_alignment(&raw mut da),
        )?;
        check(
            "hv_gic_get_redistributor_base_alignment",
            ffi::hv_gic_get_redistributor_base_alignment(&raw mut ra),
        )?;
        check(
            "hv_gic_get_redistributor_region_size",
            ffi::hv_gic_get_redistributor_region_size(&raw mut rsize),
        )?;
    }
    if !GICD.is_multiple_of(da as u64) || !GICR.is_multiple_of(ra as u64) {
        return Err(format!(
            "the GIC of the framework needs the alignments {da:#x} and {ra:#x}"
        ));
    }
    // SAFETY: the configuration is made, used and released here.
    unsafe {
        let c = ffi::hv_gic_config_create();
        if c.is_null() {
            return Err("hv_gic_config_create failed".into());
        }
        let r = check(
            "hv_gic_config_set_distributor_base",
            ffi::hv_gic_config_set_distributor_base(c, GICD),
        )
        .and_then(|()| {
            check(
                "hv_gic_config_set_redistributor_base",
                ffi::hv_gic_config_set_redistributor_base(c, GICR),
            )
        })
        .and_then(|()| check("hv_gic_create", ffi::hv_gic_create(c)));
        ffi::os_release(c);
        r
    }
}

impl Machine {
    /// Run `f` on the bus, then give the GIC each interrupt line whose level changed, and wake a
    /// vCPU that waits.
    pub fn with_bus<T>(&self, f: impl FnOnce(&mut Bus) -> T) -> T {
        let mut bus = self.bus.lock().unwrap();
        let t = f(&mut bus);
        let now = bus.irqs();
        drop(bus);
        let mut levels = self.levels.lock().unwrap();
        let mut raised = false;
        for (spi, level) in now {
            let old = levels.iter_mut().find(|(s, _)| *s == spi);
            let changed = match old {
                Some(o) if o.1 == level => false,
                Some(o) => {
                    o.1 = level;
                    true
                }
                None => {
                    levels.push((spi, level));
                    level
                }
            };
            if changed {
                // SAFETY: the SPI is in the range of the GIC; a bad one gives an error code, which
                // only means that the line is not wired.
                let r = unsafe { ffi::hv_gic_set_spi(spi, level) };
                if trace() {
                    eprintln!("9h: spi {spi} level {level}: {}", ffi::error_name(r));
                }
                raised |= level;
            }
        }
        drop(levels);
        if raised {
            self.kick();
        }
        t
    }

    /// Wake a vCPU that waits in `WFI`.
    pub fn kick(&self) {
        *self.wake.0.lock().unwrap() = true;
        self.wake.1.notify_all();
    }

    /// Wait until a kick, or at most `d`.
    fn sleep(&self, d: Duration) {
        let mut k = self.wake.0.lock().unwrap();
        if !*k {
            k = self.wake.1.wait_timeout(k, d).unwrap().0;
        }
        *k = false;
    }
}

/// A vCPU, run by the thread that made it.
struct Vcpu {
    id: ffi::hv_vcpu_t,
    exit: *const ffi::hv_vcpu_exit_t,
    sysregs: Vec<u64>,
}

impl Vcpu {
    fn new(mpidr: u64) -> Result<Vcpu, String> {
        let mut id = 0;
        let mut exit = std::ptr::null();
        // SAFETY: the framework writes the ID and the address of the exit record.
        check("hv_vcpu_create", unsafe {
            ffi::hv_vcpu_create(&raw mut id, &raw mut exit, std::ptr::null_mut())
        })?;
        let cpu = Vcpu {
            id,
            exit,
            sysregs: Vec::new(),
        };
        cpu.set_sys(ffi::HV_SYS_REG_MPIDR_EL1, mpidr)?;
        Ok(cpu)
    }

    fn reg(&self, r: u32) -> u64 {
        let mut v = 0;
        // SAFETY: writes one u64. An error leaves 0, and the register numbers are constants.
        unsafe { ffi::hv_vcpu_get_reg(self.id, r, &raw mut v) };
        v
    }

    fn set_reg(&self, r: u32, v: u64) -> Result<(), String> {
        // SAFETY: a call with plain values.
        check("hv_vcpu_set_reg", unsafe {
            ffi::hv_vcpu_set_reg(self.id, r, v)
        })
    }

    fn sys(&self, r: u16) -> u64 {
        let mut v = 0;
        // SAFETY: writes one u64.
        unsafe { ffi::hv_vcpu_get_sys_reg(self.id, r, &raw mut v) };
        v
    }

    fn set_sys(&self, r: u16, v: u64) -> Result<(), String> {
        // SAFETY: a call with plain values.
        check("hv_vcpu_set_sys_reg", unsafe {
            ffi::hv_vcpu_set_sys_reg(self.id, r, v)
        })
    }

    fn advance(&self) -> Result<(), String> {
        self.set_reg(ffi::HV_REG_PC, self.reg(ffi::HV_REG_PC) + 4)
    }

    /// Run until the next exit, and decode it.
    fn step(&mut self) -> Result<Exit, String> {
        // SAFETY: this thread made the vCPU.
        check("hv_vcpu_run", unsafe { ffi::hv_vcpu_run(self.id) })?;
        // SAFETY: the framework keeps the exit record of the vCPU valid while the vCPU exists.
        let e = unsafe { *self.exit };
        if trace() {
            let pc = self.reg(ffi::HV_REG_PC);
            eprintln!(
                "9h: exit {} esr {:#x} pa {:#x} pc {pc:#x}",
                e.reason, e.exception.syndrome, e.exception.physical_address
            );
        }
        Ok(match e.reason {
            ffi::HV_EXIT_REASON_CANCELED => Exit::Kicked,
            ffi::HV_EXIT_REASON_VTIMER_ACTIVATED => Exit::Timer,
            ffi::HV_EXIT_REASON_EXCEPTION => {
                decode(e.exception.syndrome, e.exception.physical_address, |r| {
                    self.xreg(r)
                })
            }
            _ => Exit::Other("an exit of an unknown reason", u64::from(e.reason)),
        })
    }

    fn xreg(&self, r: u8) -> u64 {
        if r == 31 { 0 } else { self.reg(u32::from(r)) }
    }

    /// Unmask the timer when the guest no longer has its interrupt asserted.
    fn timer_rearm(&self) {
        let mut masked = false;
        // SAFETY: writes one bool.
        unsafe { ffi::hv_vcpu_get_vtimer_mask(self.id, &raw mut masked) };
        if masked {
            let ctl = self.sys(ffi::HV_SYS_REG_CNTV_CTL_EL0);
            if ctl & 0b111 != 0b101 {
                // SAFETY: a call with plain values.
                unsafe { ffi::hv_vcpu_set_vtimer_mask(self.id, false) };
            }
        }
    }

    /// How long until the virtual timer fires, or None when it is off.
    fn timer_due(&self) -> Option<Duration> {
        let ctl = self.sys(ffi::HV_SYS_REG_CNTV_CTL_EL0);
        if ctl & 0b11 != 0b01 {
            return None;
        }
        let cval = self.sys(ffi::HV_SYS_REG_CNTV_CVAL_EL0);
        let mut off = 0;
        // SAFETY: writes one u64.
        unsafe { ffi::hv_vcpu_get_vtimer_offset(self.id, &raw mut off) };
        // SAFETY: no arguments.
        let now = unsafe { ffi::mach_absolute_time() }.wrapping_sub(off);
        // The counter of Apple silicon runs at 24 MHz, so a tick is 125/3 ns.
        Some(Duration::from_nanos(
            cval.saturating_sub(now).saturating_mul(125) / 3,
        ))
    }

    /// Log the state of the timer and of the interrupt controller, for `H9_TRACE`.
    fn trace_state(&self) {
        let mut masked = false;
        // SAFETY: writes one bool.
        unsafe { ffi::hv_vcpu_get_vtimer_mask(self.id, &raw mut masked) };
        let ctl = self.sys(ffi::HV_SYS_REG_CNTV_CTL_EL0);
        let g = |r: u32| {
            let mut v = 0;
            // SAFETY: writes one u64.
            unsafe { ffi::hv_gic_get_redistributor_reg(self.id, r, &raw mut v) };
            v
        };
        let icc = |r: u16| {
            let mut v = 0;
            // SAFETY: writes one u64.
            unsafe { ffi::hv_gic_get_icc_reg(self.id, r, &raw mut v) };
            v
        };
        let mut off = 0;
        // SAFETY: writes one u64.
        let r = unsafe { ffi::hv_vcpu_get_vtimer_offset(self.id, &raw mut off) };
        // SAFETY: no arguments.
        let now = unsafe { ffi::mach_absolute_time() };
        eprintln!(
            "9h: timer: cval {:#x} offset {off:#x} ({}) host {now:#x} guest {:#x}",
            self.sys(ffi::HV_SYS_REG_CNTV_CVAL_EL0),
            ffi::error_name(r),
            now.wrapping_sub(off)
        );
        let mut dctlr = 0;
        // SAFETY: writes one u64.
        unsafe { ffi::hv_gic_get_distributor_reg(0, &raw mut dctlr) };
        eprintln!(
            "9h: state: pc {:#x} cpsr {:#x} CNTV_CTL {ctl:#x} due {:?} masked {masked} isenabler0 {:#x} ispendr0 {:#x} isactiver0 {:#x} igroupr0 {:#x} pmr {:#x} igrpen1 {:#x} gicd_ctlr {dctlr:#x}",
            self.reg(ffi::HV_REG_PC),
            self.reg(ffi::HV_REG_CPSR),
            self.timer_due(),
            g(0x10100),
            g(0x10200),
            g(0x10300),
            g(0x10080),
            icc(0xc230),
            icc(0xc667)
        );
    }

    fn run_loop(&mut self, m: &Machine) -> Result<Stop, String> {
        loop {
            self.timer_rearm();
            match self.step()? {
                Exit::MmioRead {
                    pa,
                    size,
                    reg,
                    signed,
                    wide,
                } => {
                    let mut v = m.with_bus(|b| b.read(pa, size))?;
                    if signed && size < 8 {
                        let sh = 64 - 8 * u32::from(size);
                        v = ((v << sh).cast_signed() >> sh).cast_unsigned();
                    }
                    if !wide {
                        v &= 0xffff_ffff;
                    }
                    if reg != 31 {
                        self.set_reg(u32::from(reg), v)?;
                    }
                    self.advance()?;
                }
                Exit::MmioWrite { pa, size, value } => {
                    m.with_bus(|b| b.write(pa, size, value))?;
                    self.advance()?;
                }
                Exit::Hypercall(x) => match psci::call(&x) {
                    Some(Action::Return(v)) => self.set_reg(ffi::HV_REG_X0, v)?,
                    Some(Action::Off | Action::CpuOff) => return Ok(Stop::Off),
                    Some(Action::Reset) => return Ok(Stop::Reset),
                    Some(Action::CpuOn { .. }) => {
                        self.set_reg(ffi::HV_REG_X0, psci::NOT_SUPPORTED)?;
                    }
                    None => self.set_reg(ffi::HV_REG_X0, psci::NOT_SUPPORTED)?,
                },
                Exit::SysReg { read, rt, iss } => {
                    if !self.sysregs.contains(&iss) {
                        self.sysregs.push(iss);
                        eprintln!(
                            "9h: the guest used the system register {}, which reads as 0",
                            sysreg_name(iss)
                        );
                    }
                    if read && rt != 31 {
                        self.set_reg(u32::from(rt), 0)?;
                    }
                    self.advance()?;
                }
                Exit::Wait => {
                    self.advance()?;
                    if let Some(d) = self.timer_due() {
                        m.sleep(d.min(Duration::from_millis(100)));
                    } else {
                        m.sleep(Duration::from_millis(100));
                    }
                }
                Exit::Timer => {
                    // SAFETY: a call with plain values; the PPI of the timer is pending until the
                    // guest takes it.
                    let r = unsafe {
                        ffi::hv_gic_set_redistributor_reg(
                            self.id,
                            ffi::HV_GIC_REDISTRIBUTOR_REG_GICR_ISPENDR0,
                            1 << VTIMER_INTID,
                        )
                    };
                    check("hv_gic_set_redistributor_reg", r)?;
                }
                Exit::Kicked => {
                    if trace() {
                        self.trace_state();
                    }
                }
                Exit::Other(what, v) => {
                    return Err(format!(
                        "{what} ({v:#x}) at pc {:#x}, elr {:#x}, esr {:#x}, far {:#x}, vbar {:#x}, sctlr {:#x}",
                        self.reg(ffi::HV_REG_PC),
                        self.sys(ffi::HV_SYS_REG_ELR_EL1),
                        self.sys(ffi::HV_SYS_REG_ESR_EL1),
                        self.sys(ffi::HV_SYS_REG_FAR_EL1),
                        self.sys(ffi::HV_SYS_REG_VBAR_EL1),
                        self.sys(ffi::HV_SYS_REG_SCTLR_EL1),
                    ));
                }
            }
        }
    }
}

impl Drop for Vcpu {
    fn drop(&mut self) {
        // SAFETY: this thread made the vCPU, and it is not used after this.
        unsafe { ffi::hv_vcpu_destroy(self.id) };
    }
}

/// The monitor logs each exit of the vCPU when the environment has `H9_TRACE`.
fn trace() -> bool {
    static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *T.get_or_init(|| std::env::var_os("H9_TRACE").is_some())
}

/// The name of a system register from the ISS of its trap, as `S3_0_C1_C0_0`.
#[must_use]
pub fn sysreg_name(iss: u64) -> String {
    let op0 = (iss >> 20) & 3;
    let op2 = (iss >> 17) & 7;
    let op1 = (iss >> 14) & 7;
    let crn = (iss >> 10) & 15;
    let crm = (iss >> 1) & 15;
    format!("S{op0}_{op1}_C{crn}_C{crm}_{op2}")
}

/// Decode an exception exit from its syndrome. `xreg` gives the value of a general register.
pub fn decode(esr: u64, pa: u64, xreg: impl Fn(u8) -> u64) -> Exit {
    let ec = esr >> 26;
    let iss = esr & 0x1ff_ffff;
    match ec {
        0x01 => Exit::Wait,
        0x16 => Exit::Hypercall([xreg(0), xreg(1), xreg(2), xreg(3), xreg(4), xreg(5)]),
        0x18 => Exit::SysReg {
            read: iss & 1 == 1,
            rt: ((iss >> 5) & 31) as u8,
            iss: iss & !(31 << 5) & !1,
        },
        0x24 if iss & 1 << 24 != 0 => {
            let size = 1u8 << ((iss >> 22) & 3);
            let reg = ((iss >> 16) & 31) as u8;
            if iss & 1 << 6 != 0 {
                let v = xreg(reg);
                let value = if size >= 8 {
                    v
                } else {
                    v & ((1u64 << (8 * u32::from(size))) - 1)
                };
                Exit::MmioWrite { pa, size, value }
            } else {
                Exit::MmioRead {
                    pa,
                    size,
                    reg,
                    signed: iss & 1 << 21 != 0,
                    wide: iss & 1 << 15 != 0,
                }
            }
        }
        0x24 => Exit::Other("a data abort with no syndrome for the access", esr),
        0x20 => Exit::Other("an instruction abort outside the RAM", esr),
        _ => Exit::Other("an exception of an unknown class", esr),
    }
}

#[cfg(test)]
mod tests {
    use super::{decode, sysreg_name};
    use h9_vm::Exit;

    #[test]
    fn a_data_abort_is_a_load_or_a_store() {
        // STR W2, [X1]: ISV, SAS=2 (word), SRT=2, WnR.
        let esr = 0x24 << 26 | 1 << 25 | 1 << 24 | 2 << 22 | 2 << 16 | 1 << 6;
        assert_eq!(
            decode(esr, 0x0900_0000, |r| if r == 2 { 0x1_0000_0041 } else { 0 }),
            Exit::MmioWrite {
                pa: 0x0900_0000,
                size: 4,
                value: 0x41
            }
        );
        // LDR W3, [X1, #0x18].
        let esr = 0x24 << 26 | 1 << 25 | 1 << 24 | 2 << 22 | 3 << 16;
        assert_eq!(
            decode(esr, 0x0900_0018, |_| 0),
            Exit::MmioRead {
                pa: 0x0900_0018,
                size: 4,
                reg: 3,
                signed: false,
                wide: false
            }
        );
        // A store of the zero register.
        let esr = 0x24 << 26 | 1 << 24 | 31 << 16 | 1 << 6;
        assert_eq!(
            decode(esr, 0x10, |_| 7),
            Exit::MmioWrite {
                pa: 0x10,
                size: 1,
                value: 7
            }
        );
    }

    #[test]
    fn hvc_wfi_and_system_registers_are_known() {
        assert_eq!(
            decode(0x16 << 26, 0, u64::from),
            Exit::Hypercall([0, 1, 2, 3, 4, 5])
        );
        assert_eq!(decode(0x01 << 26, 0, |_| 0), Exit::Wait);
        // MRS X5, OSLSR_EL1 is S2_0_C1_C1_4.
        let iss = 2 << 20 | 4 << 17 | 1 << 10 | 5 << 5 | 1 << 1 | 1;
        let e = decode(0x18 << 26 | iss, 0, |_| 0);
        let Exit::SysReg { read, rt, iss } = e else {
            panic!("{e:?}")
        };
        assert!(read);
        assert_eq!(rt, 5);
        assert_eq!(sysreg_name(iss), "S2_0_C1_C1_4");
    }
}
