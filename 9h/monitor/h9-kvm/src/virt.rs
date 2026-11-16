//! The arm64 machine: the layout of the QEMU `virt` machine, as in the HVF back end (plan 10.3).

use h9_vm::Ram;
use kvm_bindings::{
    KVM_ARM_IRQ_TYPE_SHIFT, KVM_ARM_IRQ_TYPE_SPI, KVM_ARM_VCPU_POWER_OFF, KVM_ARM_VCPU_PSCI_0_2,
    KVM_DEV_ARM_VGIC_CTRL_INIT, KVM_DEV_ARM_VGIC_GRP_ADDR, KVM_DEV_ARM_VGIC_GRP_CTRL,
    KVM_DEV_ARM_VGIC_GRP_NR_IRQS, KVM_REG_ARM_CORE, KVM_REG_ARM64, KVM_REG_SIZE_U64,
    KVM_VGIC_V3_ADDR_TYPE_DIST, KVM_VGIC_V3_ADDR_TYPE_REDIST, kvm_create_device, kvm_device_attr,
    kvm_device_type_KVM_DEV_TYPE_ARM_VGIC_V3, kvm_vcpu_init,
};
use kvm_ioctls::{DeviceFd, Kvm, VcpuFd, VmFd};

use crate::kvm_error;

/// The name of the machine in the messages.
pub const NAME: &str = "arm64";
/// The guest physical address of the `GICv3` distributor.
pub const GICD: u64 = 0x0800_0000;
/// The guest physical address of the first `GICv3` redistributor. Each vCPU has 128 KiB.
pub const GICR: u64 = 0x080a_0000;
/// The 9h hypercalls that the SMCCC filter sends to the monitor: the first number and the count.
/// They are the SMC64 fast calls of the hypervisor vendor.
pub const H9_CALLS: (u32, u32) = (0xc600_0000, 0x1_0000);
/// The interrupts of the GIC: 32 private ones and 224 shared ones.
const NR_IRQS: u32 = 256;

/// The group and the attribute of the SMCCC filter on the VM, and the action that sends a call to
/// the monitor. They are not in `kvm-bindings` 0.14.
const KVM_ARM_VM_SMCCC_CTRL: u32 = 0;
const KVM_ARM_VM_SMCCC_FILTER: u64 = 0;
const KVM_SMCCC_FILTER_FWD_TO_USER: u8 = 2;

/// `struct kvm_smccc_filter` of Linux.
#[repr(C)]
struct SmcccFilter {
    base: u32,
    nr_functions: u32,
    action: u8,
    pad: [u8; 15],
}

/// Nothing comes before the RAM on this machine.
#[allow(clippy::unnecessary_wraps)]
pub const fn before_ram(_: &VmFd, _: &Ram) -> Result<(), String> {
    Ok(())
}

/// Make `ncpu` vCPUs with PSCI 0.2, where each vCPU but the first is off at the start, then the
/// GIC, and then the SMCCC filter.
pub fn vcpus(_: &Kvm, vm: &VmFd, ncpu: usize) -> Result<Vec<VcpuFd>, String> {
    let mut init = kvm_vcpu_init::default();
    vm.get_preferred_target(&mut init)
        .map_err(kvm_error("KVM_ARM_PREFERRED_TARGET"))?;
    let mut vcpus = Vec::new();
    for n in 0..ncpu {
        let cpu = vm
            .create_vcpu(n as u64)
            .map_err(kvm_error("KVM_CREATE_VCPU"))?;
        let mut i = init;
        i.features[0] |= 1 << KVM_ARM_VCPU_PSCI_0_2;
        if n > 0 {
            i.features[0] |= 1 << KVM_ARM_VCPU_POWER_OFF;
        }
        cpu.vcpu_init(&i).map_err(kvm_error("KVM_ARM_VCPU_INIT"))?;
        vcpus.push(cpu);
    }
    gic(vm)?;
    filter(vm)?;
    Ok(vcpus)
}

fn set(dev: &DeviceFd, group: u32, attr: u64, addr: u64, what: &str) -> Result<(), String> {
    dev.set_device_attr(&kvm_device_attr {
        flags: 0,
        group,
        attr,
        addr,
    })
    .map_err(kvm_error(what))
}

/// Make the `GICv3` of KVM at `GICD` and `GICR`, after the vCPUs, and start it.
fn gic(vm: &VmFd) -> Result<(), String> {
    let mut d = kvm_create_device {
        type_: kvm_device_type_KVM_DEV_TYPE_ARM_VGIC_V3,
        fd: 0,
        flags: 0,
    };
    let dev = vm.create_device(&mut d).map_err(|e| {
        format!("KVM_CREATE_DEVICE: {e}. The arm64 machine of 9h needs the GICv3 of KVM.")
    })?;
    let dist = GICD;
    let redist = GICR;
    let nr = NR_IRQS;
    set(
        &dev,
        KVM_DEV_ARM_VGIC_GRP_ADDR,
        u64::from(KVM_VGIC_V3_ADDR_TYPE_DIST),
        (&raw const dist) as u64,
        "the address of the GIC distributor",
    )?;
    set(
        &dev,
        KVM_DEV_ARM_VGIC_GRP_ADDR,
        u64::from(KVM_VGIC_V3_ADDR_TYPE_REDIST),
        (&raw const redist) as u64,
        "the address of the GIC redistributors",
    )?;
    set(
        &dev,
        KVM_DEV_ARM_VGIC_GRP_NR_IRQS,
        0,
        (&raw const nr) as u64,
        "the number of interrupts of the GIC",
    )?;
    set(
        &dev,
        KVM_DEV_ARM_VGIC_GRP_CTRL,
        u64::from(KVM_DEV_ARM_VGIC_CTRL_INIT),
        0,
        "the start of the GIC",
    )
}

/// Send the 9h calls to the monitor.
fn filter(vm: &VmFd) -> Result<(), String> {
    let f = SmcccFilter {
        base: H9_CALLS.0,
        nr_functions: H9_CALLS.1,
        action: KVM_SMCCC_FILTER_FWD_TO_USER,
        pad: [0; 15],
    };
    vm.set_device_attr(&kvm_device_attr {
        flags: 0,
        group: KVM_ARM_VM_SMCCC_CTRL,
        attr: KVM_ARM_VM_SMCCC_FILTER,
        addr: (&raw const f) as u64,
    })
    .map_err(|e| {
        format!(
            "KVM_ARM_VM_SMCCC_FILTER: {e}. The arm64 machine of 9h needs the SMCCC filter of KVM, which Linux 6.4 and later have."
        )
    })
}

/// The ID of the core register at the byte `offset` of `struct kvm_regs`, for `KVM_GET_ONE_REG`.
const fn core(offset: u64) -> u64 {
    KVM_REG_ARM64 | KVM_REG_SIZE_U64 | KVM_REG_ARM_CORE as u64 | (offset / 4)
}

/// The register Xn.
const fn x(n: u64) -> u64 {
    core(8 * n)
}

/// The PC, after X0 to X30 and SP in `struct user_pt_regs`.
const PC: u64 = core(8 * 32);

fn get(cpu: &VcpuFd, reg: u64) -> Result<u64, String> {
    let mut b = [0u8; 8];
    cpu.get_one_reg(reg, &mut b)
        .map_err(kvm_error("KVM_GET_ONE_REG"))?;
    Ok(u64::from_le_bytes(b))
}

fn put(cpu: &VcpuFd, reg: u64, v: u64) -> Result<(), String> {
    cpu.set_one_reg(reg, &v.to_le_bytes())
        .map(|_| ())
        .map_err(kvm_error("KVM_SET_ONE_REG"))
}

/// Start vCPU 0 at `entry` with `x0` in X0. `KVM_ARM_VCPU_INIT` has put it at EL1 with the
/// interrupts masked and the MMU off.
pub fn boot(cpu: &VcpuFd, entry: u64, x0: u64) -> Result<(), String> {
    put(cpu, PC, entry)?;
    put(cpu, x(0), x0)
}

/// The number and the arguments of a hypercall: X0 to X5. KVM does not give them in the exit on
/// arm64.
pub fn call_args(cpu: &VcpuFd) -> Result<[u64; 6], String> {
    let mut a = [0u64; 6];
    for (n, v) in a.iter_mut().enumerate() {
        *v = get(cpu, x(n as u64))?;
    }
    Ok(a)
}

/// Put the result of a hypercall in X0.
pub fn call_result(cpu: &VcpuFd, v: u64) -> Result<(), String> {
    put(cpu, x(0), v)
}

/// The line of `KVM_IRQ_LINE` for the interrupt `irq` of a device, which is the INTID of an SPI.
pub const fn irq_line(irq: u32) -> u32 {
    KVM_ARM_IRQ_TYPE_SPI << KVM_ARM_IRQ_TYPE_SHIFT | irq
}

/// The registers of a vCPU on one line, for `Machine::dump_regs`: the PC, the PSTATE, `SP_EL0`,
/// `SP_EL1` and X0 to X30. A register that does not read gives `?`.
pub fn regs_line(cpu: &VcpuFd) -> String {
    let r = |id: u64| get(cpu, id).map_or_else(|_| "?".to_string(), |v| format!("{v:#x}"));
    let xs: Vec<String> = (0..31)
        .map(|n| get(cpu, x(n)).map_or_else(|_| "?".to_string(), |v| format!("{v:x}")))
        .collect();
    format!(
        "pc {} pstate {} sp_el0 {} sp_el1 {} x0-x30 {}",
        r(PC),
        r(core(8 * 33)),
        r(core(8 * 31)),
        r(core(8 * 34)),
        xs.join(" ")
    )
}

#[cfg(test)]
mod tests {
    use super::{PC, irq_line, x};

    #[test]
    fn the_register_ids_are_those_of_linux() {
        // KVM_REG_ARM64 | KVM_REG_SIZE_U64 | KVM_REG_ARM_CORE | offset / 4.
        assert_eq!(x(0), 0x6030_0000_0010_0000);
        assert_eq!(x(5), 0x6030_0000_0010_000a);
        assert_eq!(PC, 0x6030_0000_0010_0040);
        assert_eq!(irq_line(48), 0x0100_0030);
    }
}
