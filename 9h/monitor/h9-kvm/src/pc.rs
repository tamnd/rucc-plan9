//! The amd64 machine: the PC of `9pc64` (plan 10.4).

use h9_vm::{Ram, multiboot};
use kvm_bindings::{
    KVM_MAX_CPUID_ENTRIES, KVM_PIT_SPEAKER_DUMMY, Msrs, kvm_msr_entry, kvm_pit_config, kvm_regs,
    kvm_segment,
};
use kvm_ioctls::{Kvm, VcpuFd, VmFd};

use crate::kvm_error;

/// The name of the machine in the messages.
pub const NAME: &str = "amd64";
/// The port of the 9h hypercalls.
pub const HYPERCALL_PORT: u16 = 0x509;
/// The reset control register of the PC.
pub const RESET_PORT: u16 = 0xcf9;
/// The bit of `RESET_PORT` that starts the reset.
pub const RESET_CPU: u8 = 1 << 2;
/// The three pages that KVM needs for the TSS of a real-mode guest on Intel, below the BIOS.
const TSS: usize = 0xfffb_d000;
/// The MSR of the default memory type of the MTRRs.
const MTRR_DEF_TYPE: u32 = 0x2ff;
/// The MTRRs on (bit 11), and the default type write-back (6). A BIOS sets the MTRRs, but the
/// machine has no BIOS, and at reset the MTRRs are off, which makes all memory uncacheable. The
/// kernel then removes all RAM from its memory map in `mtrrexclude` of `pc/memory.c`, because RAM
/// must be write-back, and `rampage` panics.
pub const MTRR_DEF_WRITEBACK: u64 = 1 << 11 | 6;

/// Make the devices of KVM before the RAM is mapped: the TSS pages, the interrupt controllers and
/// the timer.
pub fn before_ram(vm: &VmFd, ram: &Ram) -> Result<(), String> {
    if ram.base() != 0 {
        return Err("the RAM of the amd64 machine starts at address 0".into());
    }
    vm.set_tss_address(TSS)
        .map_err(kvm_error("KVM_SET_TSS_ADDR"))?;
    vm.create_irq_chip()
        .map_err(kvm_error("KVM_CREATE_IRQCHIP"))?;
    vm.create_pit2(kvm_pit_config {
        flags: KVM_PIT_SPEAKER_DUMMY,
        ..Default::default()
    })
    .map_err(kvm_error("KVM_CREATE_PIT2"))
}

/// Make `ncpu` vCPUs.
pub fn vcpus(kvm: &Kvm, vm: &VmFd, ncpu: usize) -> Result<Vec<VcpuFd>, String> {
    let cpuid = kvm
        .get_supported_cpuid(KVM_MAX_CPUID_ENTRIES)
        .map_err(kvm_error("KVM_GET_SUPPORTED_CPUID"))?;
    let mut vcpus = Vec::new();
    for n in 0..ncpu {
        let cpu = vm
            .create_vcpu(n as u64)
            .map_err(kvm_error("KVM_CREATE_VCPU"))?;
        // The kernel finds the APIC ID of each processor in CPUID, and it must be the one of the
        // MADT, which is the number of the processor.
        let mut id = cpuid.clone();
        for e in id.as_mut_slice() {
            match e.function {
                1 => e.ebx = e.ebx & 0x00ff_ffff | (n as u32) << 24,
                0xb | 0x1f => e.edx = n as u32,
                _ => {}
            }
        }
        cpu.set_cpuid2(&id).map_err(kvm_error("KVM_SET_CPUID2"))?;
        let msrs = Msrs::from_entries(&[kvm_msr_entry {
            index: MTRR_DEF_TYPE,
            data: MTRR_DEF_WRITEBACK,
            ..Default::default()
        }])
        .map_err(|e| format!("the MSRs of vCPU {n}: {e:?}"))?;
        let set = cpu.set_msrs(&msrs).map_err(kvm_error("KVM_SET_MSRS"))?;
        if set != 1 {
            return Err(format!(
                "KVM_SET_MSRS: vCPU {n} did not take the MTRR default type"
            ));
        }
        vcpus.push(cpu);
    }
    Ok(vcpus)
}

/// Put vCPU 0 in the state that the multiboot specification gives at the entry: 32-bit protected
/// mode with flat segments, no paging, no interrupts, and the magic number in EAX.
pub fn boot(cpu: &VcpuFd, entry: u64, info: u64) -> Result<(), String> {
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

/// The number and the arguments of a hypercall: RAX, RBX, RCX, RSI, RDI and R8.
pub fn call_args(cpu: &VcpuFd) -> Result<[u64; 6], String> {
    let r = cpu.get_regs().map_err(kvm_error("KVM_GET_REGS"))?;
    Ok([r.rax, r.rbx, r.rcx, r.rsi, r.rdi, r.r8])
}

/// Put the result of a hypercall in RAX. KVM moves RIP past the `OUT` when the vCPU runs again.
pub fn call_result(cpu: &VcpuFd, v: u64) -> Result<(), String> {
    let mut r = cpu.get_regs().map_err(kvm_error("KVM_GET_REGS"))?;
    r.rax = v;
    cpu.set_regs(&r).map_err(kvm_error("KVM_SET_REGS"))
}

/// The line of `KVM_IRQ_LINE` for the interrupt `irq` of a device: the GSI, which is the ISA IRQ.
pub const fn irq_line(irq: u32) -> u32 {
    irq
}
