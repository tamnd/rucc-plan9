//! `r9 objcheck` for arm64: check the records of a `.7` object against the table of `7l` (plan
//! 07.6, row arm64).
//!
//! `oplook` in `7l/span.c` gives each operand a class with `aclass`, and takes the first entry of
//! `optab` for the opcode whose classes cover the classes of the operands. The middle register
//! has the class `C_REG`, or `C_FREG` for some opcodes, when it is there. When no entry covers the
//! classes, the link stops with "illegal combination". This module does the same for each record
//! of an object, with the table of `optab7.rs`. The check is the same as the check that rucc does
//! when it writes a `.7` object, so that the two find the same records.
//!
//! Before `span`, `7l` changes some records. `ldobj` makes an `ADD` of a negative constant a
//! `SUB` of the positive constant, and the reverse, and it puts a floating constant of `FMOVS` and
//! `FMOVD` in memory when the instruction cannot hold it. `patch` makes the target of `B`, `BL`
//! and `RET` a branch when it names a symbol. `noop` makes each `RETURN` the release of the frame,
//! so the check does not look at `RETURN`, and it sets the size of the frame, which gives the
//! offsets of the `AUTO` and `PARAM` operands. The check makes the same changes.
//!
//! The class of an operand that names a symbol depends on the address of the symbol, which only
//! the linker knows. The check finds each class that the operand can have, and the record must
//! have an entry for each of them.

// The masks are the ones of `7l/span.c`, so that the two read the same.
#![allow(clippy::verbose_bit_mask)]

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::obj::{Addr, Object, Prog, d7};
use crate::objcheck::Bad;
use crate::optab7 as t;

/// The records that `span` does not encode, or that `noop` replaces.
const PSEUDO: &[&str] = &[
    "TEXT", "DATA", "GLOBL", "HISTORY", "END", "NAME", "SIGNAME", "DYNT", "INIT", "NOP", "RETURN",
];

/// The opcodes whose middle register is a floating register, the `asregclass` of `7l`.
const FREG_MIDDLE: &[&str] = &[
    "FMADDD", "FMADDS", "FMSUBD", "FMSUBS", "FNMADDS", "FNMADDD", "FNMSUBS", "FNMSUBD", "MOVPS",
    "MOVPD", "MOVPQ",
];

/// The register that is zero, or the stack pointer.
const REGZERO: u8 = 31;

/// `D_OCONST` of `7.out.h`, which `obj` does not name.
const OCONST: u8 = 20;

/// The size of the return address, `PCSZ` of `7l`.
const PCSZ: i64 = 8;

/// The alignment of the frame, `STACKALIGN` of `7l`.
const STACKALIGN: i64 = 16;

/// The classes that an offset from the static base can have, the `sextclass` of `7l`.
const EXTERN: &[&str] = &[
    "C_SEXT1", "C_SEXT2", "C_SEXT4", "C_SEXT8", "C_SEXT16", "C_LEXT",
];

/// The classes of an offset from a register, the `oregclass` of `7l`.
const OREG: [&str; 11] = [
    "C_ZOREG",
    "C_NSOREG",
    "C_NPOREG",
    "C_PSOREG",
    "C_PPOREG",
    "C_UOREG4K",
    "C_UOREG8K",
    "C_UOREG16K",
    "C_UOREG32K",
    "C_UOREG64K",
    "C_LOREG",
];

/// The classes of an offset in the frame, the `autoclass` of `7l`. An offset of zero is
/// `C_PSAUTO`, as in `7l`.
const AUTO: [&str; 11] = [
    "C_PSAUTO",
    "C_NSAUTO",
    "C_NPAUTO",
    "C_PSAUTO",
    "C_PPAUTO",
    "C_UAUTO4K",
    "C_UAUTO8K",
    "C_UAUTO16K",
    "C_UAUTO32K",
    "C_UAUTO64K",
    "C_LAUTO",
];

/// The table of `7l`, ready for the check.
pub struct Checker {
    /// `cover[a][b]`: class `a` of an entry covers class `b` of an operand, the `cmp` of `7l`.
    cover: Vec<Vec<bool>>,
    /// The classes of the entries of each opcode name, after the ranges of `buildop`.
    entries: HashMap<&'static str, Vec<(u8, u8, u8)>>,
    /// The value of each class name.
    class: HashMap<&'static str, u8>,
}

/// The table, made once.
pub static CHECKER: LazyLock<Checker> = LazyLock::new(Checker::new);

/// The position of `name` in the classes, or `C_GOK`.
fn index(name: &str) -> usize {
    t::CLASSES
        .iter()
        .position(|c| *c == name)
        .unwrap_or(t::CLASSES.len() - 1)
}

/// Class `a` of an entry covers class `b` of an operand: `cmp` in `7l/span.c`.
fn cmp(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let within = |lo: &str, hi: &str| (index(lo)..=index(hi)).contains(&index(b));
    match a {
        "C_RSP" => b == "C_REG",
        "C_REG" | "C_ADDCON0" => b == "C_ZCON",
        "C_ADDCON" => matches!(b, "C_ZCON" | "C_ADDCON0" | "C_ABCON"),
        "C_BITCON32" | "C_BITCON64" => matches!(b, "C_BITCON" | "C_ABCON" | "C_MBCON"),
        "C_BITCON" => matches!(b, "C_ABCON" | "C_MBCON"),
        "C_MOVCON" => matches!(b, "C_MBCON" | "C_ZCON" | "C_ADDCON0"),
        "C_LCON" => matches!(
            b,
            "C_ZCON"
                | "C_BITCON"
                | "C_BITCON32"
                | "C_BITCON64"
                | "C_ADDCON"
                | "C_ADDCON0"
                | "C_ABCON"
                | "C_MBCON"
                | "C_MOVCON"
        ),
        "C_VCON" => cmp("C_LCON", b),
        "C_LACON" => b == "C_AACON",
        "C_SEXT2" => b == "C_SEXT1",
        "C_SEXT4" => within("C_SEXT1", "C_SEXT2"),
        "C_SEXT8" => within("C_SEXT1", "C_SEXT4"),
        "C_SEXT16" => within("C_SEXT1", "C_SEXT8"),
        "C_LEXT" => within("C_SEXT1", "C_SEXT16"),
        "C_PPAUTO" => b == "C_PSAUTO",
        "C_UAUTO4K" => matches!(b, "C_PSAUTO" | "C_PPAUTO"),
        "C_UAUTO8K" => cmp("C_UAUTO4K", b),
        "C_UAUTO16K" => cmp("C_UAUTO8K", b),
        "C_UAUTO32K" => cmp("C_UAUTO16K", b),
        "C_UAUTO64K" => cmp("C_UAUTO32K", b),
        "C_NPAUTO" => cmp("C_NSAUTO", b),
        "C_LAUTO" => cmp("C_NPAUTO", b) || cmp("C_UAUTO64K", b),
        "C_PSOREG" => b == "C_ZOREG",
        "C_PPOREG" => matches!(b, "C_ZOREG" | "C_PSOREG"),
        "C_UOREG4K" => {
            matches!(
                b,
                "C_ZOREG" | "C_PSAUTO" | "C_PSOREG" | "C_PPAUTO" | "C_PPOREG"
            )
        }
        "C_UOREG8K" => cmp("C_UOREG4K", b),
        "C_UOREG16K" => cmp("C_UOREG8K", b),
        "C_UOREG32K" => cmp("C_UOREG16K", b),
        "C_UOREG64K" => cmp("C_UOREG32K", b),
        "C_NPOREG" => cmp("C_NSOREG", b),
        "C_LOREG" => cmp("C_NPOREG", b) || cmp("C_UOREG64K", b),
        "C_LBRA" => b == "C_SBRA",
        _ => false,
    }
}

/// The index of the range of `constclass` in `7l` for the offset `l`.
fn constclass(l: i64) -> usize {
    match l {
        0 => 0,
        -256..0 => 1,
        -512..0 if l & 7 == 0 => 2,
        _ if l < 0 => 10,
        1..=255 => 3,
        _ if l <= 504 && l & 7 == 0 => 4,
        _ if l <= 4095 => 5,
        _ if l <= 8190 && l & 1 == 0 => 6,
        _ if l <= 16380 && l & 3 == 0 => 7,
        _ if l <= 32760 && l & 7 == 0 => 8,
        _ if l <= 65520 && l & 0xf == 0 => 9,
        _ => 10,
    }
}

/// The constant fits the 12 bits of an `ADD`, shifted by 0 or 12: `isaddcon`.
fn isaddcon(v: i64) -> bool {
    if v < 0 {
        return false;
    }
    let v = if v & 0xfff == 0 { v >> 12 } else { v };
    v <= 0xfff
}

/// The number of ones in each element when `v` is a logical immediate of 64 bits, the `findmask`
/// of `7l`, which looks `v` up in the table of `bits.c`.
fn bitmask(v: u64) -> Option<u32> {
    if v == 0 || v == u64::MAX {
        return None;
    }
    for size in [2_u32, 4, 8, 16, 32, 64] {
        let mask = if size == 64 {
            u64::MAX
        } else {
            (1 << size) - 1
        };
        let elem = v & mask;
        let mut whole = 0_u64;
        let mut at = 0;
        while at < 64 {
            whole |= elem << at;
            at += size;
        }
        if whole != v {
            continue;
        }
        let ones = elem.count_ones();
        if ones == 0 || ones == size {
            return None;
        }
        let run = (1_u64 << ones) - 1;
        let rotr = |x: u64, r: u32| match r {
            0 => x,
            _ => ((x >> r) | (x << (size - r))) & mask,
        };
        return (0..size).any(|r| rotr(elem, r) == run).then_some(ones);
    }
    None
}

/// The field of the one 16-bit part of `v` that is not zero, `movcon` of `7l`.
fn movcon(v: u64) -> Option<u32> {
    (0..4).find(|s| v & !(0xffff_u64 << (16 * s)) == 0)
}

/// The class of a constant with no symbol.
fn constant(v: i64) -> &'static str {
    let u = u64::from_ne_bytes(v.to_ne_bytes());
    let bitcon = u >> 32 == 0 && bitmask(u).is_some_and(|s| s < 32);
    if v == 0 {
        return "C_ZCON";
    }
    if isaddcon(v) {
        return if bitcon {
            "C_ABCON"
        } else if v <= 0xfff {
            "C_ADDCON0"
        } else {
            "C_ADDCON"
        };
    }
    if movcon(u).is_some() || movcon(!u).is_some() {
        return if bitcon { "C_MBCON" } else { "C_MOVCON" };
    }
    if bitcon {
        "C_BITCON"
    } else if bitmask(u).is_some() {
        "C_BITCON64"
    } else if u >> 32 == 0 && bitmask(u | u << 32).is_some() {
        "C_BITCON32"
    } else {
        "C_LCON"
    }
}

/// The constant fits the immediate of `FMOV`: `chipfloat` of `7l`.
fn chipfloat(bits: u64) -> bool {
    let (l, h) = (bits & 0xffff_ffff, bits >> 32);
    if l != 0 || h & 0xffff != 0 {
        return false;
    }
    let n = h >> 16;
    matches!((n >> 6) & 0x1ff, 0x100 | 0xff)
}

/// The size of the frame of a function, as `noop` in `7l/noop.c` sets it: the size of `TEXT`
/// and the return address, aligned to 16, or 0 for a leaf with no locals.
fn frame(text_size: i64, leaf: bool) -> i64 {
    let mut size = if text_size < 0 { 0 } else { text_size + PCSZ };
    if leaf && size <= PCSZ {
        size = 0;
    } else if size & (STACKALIGN - 1) != 0 {
        size += STACKALIGN - (size & (STACKALIGN - 1));
    }
    size
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    /// Make the table from `optab7.rs`, as `buildop` in `7l/span.c` does.
    #[must_use]
    pub fn new() -> Self {
        let n = t::CLASSES.len();
        let mut cover = vec![vec![false; n]; n];
        for (i, row) in cover.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = cmp(t::CLASSES[i], t::CLASSES[j]);
            }
        }
        let mut entries: HashMap<&str, Vec<(u8, u8, u8)>> = HashMap::new();
        for &(op, a1, a2, a3, _, _) in t::OPTAB {
            entries.entry(op).or_default().push((a1, a2, a3));
        }
        for &(op, base) in t::RANGES {
            if let Some(e) = entries.get(base).cloned() {
                entries.insert(op, e);
            }
        }
        let class = t::CLASSES
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((*c, u8::try_from(i).ok()?)))
            .collect();
        Self {
            cover,
            entries,
            class,
        }
    }

    /// The value of the class `name`.
    fn y(&self, name: &str) -> u8 {
        self.class.get(name).copied().unwrap_or(0)
    }

    /// Each class that the operand can have in a frame of `autosize` bytes: `aclass` of `7l`.
    fn aclass(a: &Addr, autosize: i64) -> Vec<&'static str> {
        let one = |c: &'static str| vec![c];
        let symbol = matches!(a.name, d7::EXTERN | d7::STATIC);
        let stack = match a.name {
            d7::AUTO => Some(autosize + a.offset),
            d7::PARAM => Some(autosize + a.offset + PCSZ),
            _ => None,
        };
        match a.kind {
            d7::NONE => one("C_NONE"),
            d7::REG => one("C_REG"),
            d7::VREG => one("C_VREG"),
            d7::SP => one("C_RSP"),
            d7::COND => one("C_COND"),
            d7::SHIFT => one("C_SHIFT"),
            d7::EXTREG => one("C_EXTREG"),
            d7::ROFF => one("C_ROFF"),
            d7::XPOST => one("C_XPOST"),
            d7::XPRE => one("C_XPRE"),
            d7::FREG => one("C_FREG"),
            d7::SPR => one("C_SPR"),
            d7::FCONST => one("C_FCON"),
            d7::BRANCH => one("C_SBRA"),
            d7::OREG if symbol => EXTERN.to_vec(),
            d7::OREG if a.name == d7::NONE => one(OREG[constclass(a.offset)]),
            d7::OREG => stack.map_or_else(|| one("C_GOK"), |v| one(AUTO[constclass(v)])),
            OCONST if symbol => one("C_LCON"),
            d7::CONST if symbol => vec!["C_AECON", "C_LCON"],
            d7::CONST if a.name == d7::NONE => {
                if a.reg != d7::NREG && a.reg != REGZERO {
                    one(if isaddcon(a.offset) {
                        "C_AACON"
                    } else {
                        "C_LACON"
                    })
                } else {
                    one(constant(a.offset))
                }
            }
            d7::CONST => stack.map_or_else(
                || one("C_GOK"),
                |v| one(if isaddcon(v) { "C_AACON" } else { "C_LACON" }),
            ),
            _ => one("C_GOK"),
        }
    }

    /// Check one record in a frame of `autosize` bytes. Return the classes of its operands and
    /// why `7l` cannot encode it, or `None` when it can.
    #[must_use]
    pub fn check(&self, name: &str, p: &Prog, autosize: i64) -> Option<(u8, u8, u8, &'static str)> {
        if PSEUDO.contains(&name) {
            return None;
        }
        let mut name = name;
        let mut from = p.from.clone();
        let mut to = p.to.clone();
        if from.kind == d7::CONST && from.name == d7::NONE && from.offset < 0 {
            let other = match name {
                "SUB" => "ADD",
                "SUBW" => "ADDW",
                "SUBS" => "ADDS",
                "SUBSW" => "ADDSW",
                "ADD" => "SUB",
                "ADDW" => "SUBW",
                "ADDS" => "SUBS",
                "ADDSW" => "SUBSW",
                _ => name,
            };
            if other != name {
                name = other;
                from.offset = from.offset.wrapping_neg();
            }
        }
        if name == "FCVTDS" && from.kind == d7::FCONST {
            name = "FMOVS";
        }
        if matches!(name, "FMOVS" | "FMOVD")
            && from.kind == d7::FCONST
            && !chipfloat(from.fconst.unwrap_or(0))
        {
            from.kind = d7::OREG;
            from.name = d7::EXTERN;
            from.fconst = None;
        }
        if matches!(name, "B" | "BL" | "RET" | "ERET") && to.kind != d7::BRANCH && to.sym.is_some()
        {
            to.kind = d7::BRANCH;
        }
        let middle = if p.reg == d7::NREG {
            "C_NONE"
        } else if FREG_MIDDLE.contains(&name) {
            "C_FREG"
        } else {
            "C_REG"
        };
        let a2 = self.y(middle);
        let (froms, tos) = (Self::aclass(&from, autosize), Self::aclass(&to, autosize));
        let Some(entries) = self.entries.get(name) else {
            return Some((
                self.y(froms[0]),
                a2,
                self.y(tos[0]),
                "the instruction has no entry in optab",
            ));
        };
        for f in &froms {
            for t in &tos {
                let (a1, a3) = (self.y(f), self.y(t));
                let covers = |&(e1, e2, e3): &(u8, u8, u8)| {
                    (e2 == a2 || self.cover[usize::from(e2)][usize::from(a2)])
                        && self.cover[usize::from(e1)][usize::from(a1)]
                        && self.cover[usize::from(e3)][usize::from(a3)]
                };
                if !entries.iter().any(covers) {
                    return Some((
                        a1,
                        a2,
                        a3,
                        "no entry of optab covers the classes of its operands",
                    ));
                }
            }
        }
        None
    }

    /// Check each record of an object.
    #[must_use]
    pub fn object(&self, o: &Object) -> Vec<Bad> {
        let mut out = Vec::new();
        let mut function = String::from("(before any TEXT)");
        let mut text_pc = 0;
        let mut autosize = 0;
        for (i, p) in o.progs.iter().enumerate() {
            let name = o.arch.name(p.op);
            if name == "TEXT" {
                function = p
                    .from
                    .sym
                    .as_ref()
                    .map_or_else(|| "?".into(), |s| s.name.clone());
                text_pc = p.pc.unwrap_or(0);
                let leaf = !o.progs[i + 1..]
                    .iter()
                    .map(|q| o.arch.name(q.op))
                    .take_while(|n| n != "TEXT")
                    .any(|n| n == "BL");
                autosize = frame(p.to.offset, leaf);
                continue;
            }
            if let Some((a1, a2, a3, why)) = self.check(&name, p, autosize) {
                let class = |c: u8| t::CLASSES[usize::from(c)];
                let from = if p.reg == d7::NREG {
                    class(a1).to_string()
                } else {
                    format!("{}, {}", class(a1), class(a2))
                };
                out.push(Bad {
                    function: function.clone(),
                    text: o.prog(p, text_pc),
                    from,
                    to: class(a3).into(),
                    why: why.into(),
                });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obj::Sym;

    fn addr(kind: u8) -> Addr {
        Addr {
            kind,
            reg: d7::NREG,
            name: d7::NONE,
            ..Addr::default()
        }
    }

    fn reg(r: u8) -> Addr {
        Addr {
            reg: r,
            ..addr(d7::REG)
        }
    }

    fn konst(v: i64) -> Addr {
        Addr {
            offset: v,
            ..addr(d7::CONST)
        }
    }

    fn oreg(r: u8, offset: i64) -> Addr {
        Addr {
            reg: r,
            offset,
            ..addr(d7::OREG)
        }
    }

    fn sym(kind: u8, s: &str) -> Addr {
        Addr {
            name: d7::EXTERN,
            sym: Some(Sym {
                name: s.into(),
                kind: d7::EXTERN,
            }),
            ..addr(kind)
        }
    }

    fn prog(from: Addr, middle: u8, to: Addr) -> Prog {
        Prog {
            op: 0,
            line: 0,
            pc: None,
            reg: middle,
            nosched: false,
            from,
            from3: None,
            to,
        }
    }

    #[test]
    fn constants_get_the_classes_of_7l() {
        assert_eq!(constant(0), "C_ZCON");
        assert_eq!(constant(1), "C_ABCON");
        assert_eq!(constant(5), "C_ADDCON0");
        assert_eq!(constant(0x1000), "C_ABCON");
        assert_eq!(constant(0x5000), "C_ADDCON");
        assert_eq!(constant(0x1200_0000), "C_MOVCON");
        assert_eq!(constant(0xff00_0000), "C_MBCON");
        assert_eq!(constant(-1), "C_MOVCON");
        assert_eq!(constant(0x00ff_ff00), "C_BITCON");
        assert_eq!(constant(0x5555_5555_5555_5555), "C_BITCON64");
        assert_eq!(constant(0x0f0f_0f0f), "C_BITCON32");
        assert_eq!(constant(0x1234_5678), "C_LCON");
        assert_eq!(OREG[constclass(-8)], "C_NSOREG");
        assert_eq!(OREG[constclass(4096)], "C_UOREG8K");
        assert_eq!(OREG[constclass(4097)], "C_LOREG");
        assert!(chipfloat(1.0_f64.to_bits()) && chipfloat(0.5_f64.to_bits()));
        assert!(!chipfloat(0.1_f64.to_bits()) && !chipfloat(0.0_f64.to_bits()));
        for c in OREG.iter().chain(AUTO.iter()).chain(EXTERN) {
            assert!(t::CLASSES.contains(c), "{c} is not a class of 7l");
        }
    }

    #[test]
    fn the_frame_is_the_one_of_noop() {
        assert_eq!(frame(0, true), 0);
        assert_eq!(frame(0, false), 16);
        assert_eq!(frame(8, true), 16);
        assert_eq!(frame(24, false), 32);
        assert_eq!(frame(-4, false), 0);
    }

    #[test]
    fn the_cover_is_the_cmp_of_7l() {
        assert!(cmp("C_LCON", "C_MOVCON"));
        assert!(cmp("C_VCON", "C_ZCON"));
        assert!(cmp("C_LAUTO", "C_NSAUTO"));
        assert!(cmp("C_UOREG64K", "C_UOREG4K"));
        assert!(cmp("C_LEXT", "C_SEXT1"));
        assert!(!cmp("C_SEXT1", "C_LEXT"));
        assert!(!cmp("C_ZCON", "C_REG"));
    }

    #[test]
    fn common_instructions_pass_and_bad_ones_fail() {
        let c = &*CHECKER;
        let ok = |name: &str, p: Prog| {
            let r = c.check(name, &p, 32);
            assert!(r.is_none(), "{name} {p:?}: {r:?}");
        };
        let n = d7::NREG;
        ok("MOV", prog(konst(1), n, reg(0)));
        ok("MOV", prog(konst(0x1234_5678_9abc), n, reg(0)));
        ok("ADD", prog(konst(-16), n, reg(1)));
        ok("ADD", prog(reg(2), 1, reg(3)));
        ok("MOV", prog(oreg(31, 8), n, reg(0)));
        ok("MOV", prog(oreg(1, 100_000), n, reg(0)));
        ok("MOVW", prog(reg(0), n, oreg(31, 12)));
        ok("MOV", prog(sym(d7::OREG, "x"), n, reg(0)));
        ok("MOV", prog(reg(0), n, sym(d7::OREG, "x")));
        ok("MOV", prog(sym(d7::CONST, "x"), n, reg(0)));
        ok("BL", prog(addr(d7::NONE), n, sym(d7::OREG, "print")));
        let mut auto = oreg(n, -8);
        auto.name = d7::AUTO;
        ok("MOV", prog(auto.clone(), n, reg(0)));
        auto.kind = d7::CONST;
        ok("MOV", prog(auto, n, reg(0)));
        let mut f = addr(d7::FCONST);
        f.fconst = Some(0.1_f64.to_bits());
        ok("FMOVD", prog(f.clone(), n, addr(d7::FREG)));
        f.fconst = Some(2.0_f64.to_bits());
        ok("FMOVD", prog(f, n, addr(d7::FREG)));
        ok("RETURN", prog(addr(d7::NONE), n, addr(d7::NONE)));
        let (_, _, a3, why) = c
            .check("ADD", &prog(reg(0), 1, oreg(2, 0)), 0)
            .expect("ADD cannot write memory");
        assert_eq!(t::CLASSES[usize::from(a3)], "C_ZOREG");
        assert_eq!(why, "no entry of optab covers the classes of its operands");
        assert!(c.check("MOV", &prog(konst(1), n, oreg(0, 0)), 0).is_some());
    }
}
