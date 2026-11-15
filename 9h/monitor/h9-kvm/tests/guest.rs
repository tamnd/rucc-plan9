//! Small multiboot guests on KVM. The first checks the entry state, reads the CMOS through the
//! ports, writes to the console with a hypercall, and turns the machine off. The second resets the
//! machine through port 0xcf9, as `archreset` of the tree does. The tests need `/dev/kvm`. With no
//! KVM they pass with a note, unless `H9_NEED_KVM=1` is set, as in CI.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::io::Write;
use std::sync::{Arc, Mutex};

use h9_dev::cmos::{self, Cmos};
use h9_dev::h9cons::H9Cons;
use h9_dev::{Bus, PORTS, h9};
use h9_kvm::{Stop, Vm};
use h9_vm::{Ram, acpi, multiboot};

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

const MESSAGE: &[u8] = b"hello, amd64";

/// A kernel: a multiboot header at 0x100000 with the load addresses, the code at 0x100020, and
/// the message at 0x100100.
fn kernel(code: &[u8]) -> Vec<u8> {
    let mut k = vec![0u8; 0x100 + MESSAGE.len()];
    let flags = 1u32 << 16;
    let header = [
        multiboot::HEADER_MAGIC,
        flags,
        0u32.wrapping_sub(multiboot::HEADER_MAGIC.wrapping_add(flags)),
        0x10_0000,
        0x10_0000,
        0,
        0,
        0x10_0020,
    ];
    for (n, w) in header.iter().enumerate() {
        k[n * 4..n * 4 + 4].copy_from_slice(&w.to_le_bytes());
    }
    k[0x20..0x20 + code.len()].copy_from_slice(code);
    k[0x100..].copy_from_slice(MESSAGE);
    k
}

/// The kernel of the first test.
fn guest() -> Vec<u8> {
    #[rustfmt::skip]
    let code: &[u8] = &[
        0x3d, 0x02, 0xb0, 0xad, 0x2b,       // 20: cmp eax, 0x2badb002
        0x75, 0x2f,                         // 25: jne bad
        0x81, 0xfb, 0x00, 0x80, 0x00, 0x00, // 27: cmp ebx, 0x8000
        0x75, 0x27,                         // 2d: jne bad
        0xb0, 0x0d,                         // 2f: mov al, 0x0d
        0xe6, 0x70,                         // 31: out 0x70, al
        0xe4, 0x71,                         // 33: in al, 0x71
        0x3c, 0x80,                         // 35: cmp al, 0x80
        0x75, 0x1d,                         // 37: jne bad
        0xb8, 0x02, 0x00, 0x00, 0xc6,       // 39: mov eax, H9consputs
        0xbb, 0x00, 0x01, 0x10, 0x00,       // 3e: mov ebx, 0x100100
        0xb9, 0x0c, 0x00, 0x00, 0x00,       // 43: mov ecx, 12
        0x66, 0xba, 0x09, 0x05,             // 48: mov dx, 0x509
        0xee,                               // 4c: out dx, al
        0xb8, 0x08, 0x00, 0x00, 0x84,       // 4d: mov eax, SYSTEM_OFF
        0xee,                               // 52: out dx, al
        0xf4,                               // 53: hlt
        0xeb, 0xfd,                         // 54: jmp 53
        0xb8, 0x09, 0x00, 0x00, 0x84,       // 56: bad: mov eax, SYSTEM_RESET
        0x66, 0xba, 0x09, 0x05,             // 5b: mov dx, 0x509
        0xee,                               // 5f: out dx, al
        0xf4,                               // 60: hlt
        0xeb, 0xfd,                         // 61: jmp 60
    ];
    kernel(code)
}

/// True when the test can run. With no KVM, the test passes with a note, unless `H9_NEED_KVM=1`.
fn have_kvm() -> bool {
    if std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .is_ok()
    {
        return true;
    }
    assert!(
        std::env::var("H9_NEED_KVM").as_deref() != Ok("1"),
        "H9_NEED_KVM=1, and /dev/kvm does not open"
    );
    eprintln!("no /dev/kvm: the guest does not run");
    false
}

/// Run `k` on 2 vCPUs, with the console and the CMOS. The result is the stop and the output.
fn run(k: &[u8]) -> (Stop, Vec<u8>) {
    let image = multiboot::load(k).unwrap();
    let mem = 64u64 << 20;
    let ram = Arc::new(Ram::new(0, mem as usize).unwrap());
    for (pa, seg) in &image.segments {
        assert!(ram.write(*pa, seg));
    }
    let info = multiboot::info(&multiboot::Info {
        mem,
        cmdline: "console=0",
        ini: None,
    })
    .unwrap();
    assert!(ram.write(multiboot::INFO, &info));
    assert!(ram.write(acpi::RSDP, &acpi::tables(2).unwrap()));
    let out = Out::default();
    let mut bus = Bus::new();
    bus.add_hyper(Box::new(H9Cons::new(h9::CONS_IRQ, Box::new(out.clone()))));
    bus.add(PORTS + cmos::PORT, 2, Box::new(Cmos::new()));
    let vm = Vm::new(ram, bus, 2).unwrap();
    let stop = vm.run(image.entry, multiboot::INFO).unwrap();
    let text = out.0.lock().unwrap().clone();
    (stop, text)
}

#[test]
fn a_multiboot_guest_writes_and_turns_off() {
    if !have_kvm() {
        return;
    }
    let (stop, text) = run(&guest());
    assert_eq!(
        stop,
        Stop::Off,
        "Reset means that a check of the guest failed"
    );
    assert_eq!(text, MESSAGE);
}

#[test]
fn a_write_of_6_to_port_cf9_resets_the_machine() {
    if !have_kvm() {
        return;
    }
    #[rustfmt::skip]
    let code: &[u8] = &[
        0xb0, 0x02,                         // 20: mov al, 2
        0x66, 0xba, 0xf9, 0x0c,             // 22: mov dx, 0xcf9
        0xee,                               // 26: out dx, al
        0xb0, 0x06,                         // 27: mov al, 6
        0xee,                               // 29: out dx, al
        0xf4,                               // 2a: hlt
        0xeb, 0xfd,                         // 2b: jmp 2a
    ];
    let (stop, text) = run(&kernel(code));
    assert_eq!(stop, Stop::Reset);
    assert!(text.is_empty());
}
