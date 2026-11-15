//! A small guest on KVM on arm64. It writes to the console with the 9h hypercall `H9consputs`,
//! which the SMCCC filter sends to the monitor, and turns the machine off with PSCI `SYSTEM_OFF`,
//! which KVM does itself. The test needs `/dev/kvm`. With no KVM it passes with a note, unless
//! `H9_NEED_KVM=1` is set.
#![cfg(all(target_os = "linux", target_arch = "aarch64"))]

use std::io::Write;
use std::sync::{Arc, Mutex};

use h9_dev::h9cons::H9Cons;
use h9_dev::{Bus, h9};
use h9_kvm::{Stop, Vm};
use h9_vm::Ram;

#[derive(Clone, Default)]
struct Out(Arc<Mutex<Vec<u8>>>);

impl Write for Out {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const MESSAGE: &[u8] = b"hello, arm64";
const RAM: u64 = 0x4000_0000;
const ENTRY: u64 = RAM + 0x8_0000;

#[test]
fn a_guest_writes_with_hvc_and_turns_off() {
    if std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_err()
    {
        assert!(
            std::env::var("H9_NEED_KVM").as_deref() != Ok("1"),
            "H9_NEED_KVM=1, and /dev/kvm does not open"
        );
        eprintln!("no /dev/kvm: the guest does not run");
        return;
    }
    #[rustfmt::skip]
    let code: [u32; 9] = [
        0xd280_0040, // 00: movz x0, #0x2
        0xf2b8_c000, // 04: movk x0, #0xc600, lsl #16: H9consputs
        0x1000_07c1, // 08: adr x1, 0x100: the message
        0xd280_0182, // 0c: movz x2, #12
        0xd400_0002, // 10: hvc #0
        0xd280_0100, // 14: movz x0, #0x8
        0xf2b0_8000, // 18: movk x0, #0x8400, lsl #16: SYSTEM_OFF
        0xd400_0002, // 1c: hvc #0
        0x1400_0000, // 20: b .
    ];
    let ram = Arc::new(Ram::new(RAM, 64 << 20).unwrap());
    let bytes: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
    assert!(ram.write(ENTRY, &bytes));
    assert!(ram.write(ENTRY + 0x100, MESSAGE));
    let out = Out::default();
    let mut bus = Bus::new();
    bus.add_hyper(Box::new(H9Cons::new(h9::CONS_SPI, Box::new(out.clone()))));
    let vm = Vm::new(ram, bus, 2).unwrap();
    let stop = vm.run(ENTRY, 0).unwrap();
    assert_eq!(stop, Stop::Off);
    assert_eq!(out.0.lock().unwrap().as_slice(), MESSAGE);
}
