//! `r9 objcheck`: check the instructions of a `.6` object against the tables of `6l` (plan 07.6).
//!
//! `doasm` in `6l/span.c` gives each operand a class with `oclass`, and looks for a pair of the
//! `ytab` of the opcode whose classes cover the two classes. When there is none, it tries
//! `ymovtab`. When that fails too, the link stops with "doasm: notfound". This module does the
//! same for each instruction of an object, with the tables of `optab6.rs`, so that an
//! instruction that `6l` cannot encode is found on the host and not in a guest link.
//!
//! Before `span`, `6l` changes some operands. `ldobj` puts a floating constant of the single and
//! double precision instructions in memory, and `patch` makes the target of `CALL`, `RET` and
//! `JMPF` a branch when it names a symbol. `dostkoff` makes a `RET` with a constant source, which
//! is a tail call, an `ADJSP` and a `JMP` to its target. The check makes the same changes. The
//! other changes of `dostkoff` are to the offsets of the stack operands, which do not change
//! their classes.

use std::collections::HashMap;
use std::fmt::Write as _;

use crate::obj::{self, Addr, Arch, Object, Prog};
use crate::optab6 as t;

/// The operand types of amd64, from `6.out.h`.
mod d {
    pub const AL: u8 = 0;
    pub const CL: u8 = 1;
    pub const DL: u8 = 2;
    pub const BL: u8 = 3;
    pub const BPB: u8 = 5;
    pub const R15B: u8 = 15;
    pub const AX: u8 = 16;
    pub const CX: u8 = 17;
    pub const DX: u8 = 18;
    pub const BX: u8 = 19;
    pub const SP: u8 = 20;
    pub const R15: u8 = 31;
    pub const AH: u8 = 32;
    pub const BH: u8 = 35;
    pub const F0: u8 = 36;
    pub const M0: u8 = 44;
    pub const X0: u8 = 52;
    pub const CS: u8 = 68;
    pub const TASK: u8 = 78;
    pub const CR: u8 = 79;
    pub const DR: u8 = 95;
    pub const TR: u8 = 103;
    pub const NONE: u8 = 111;
    pub const BRANCH: u8 = 112;
    pub const EXTERN: u8 = 113;
    pub const STATIC: u8 = 114;
    pub const AUTO: u8 = 115;
    pub const PARAM: u8 = 116;
    pub const CONST: u8 = 117;
    pub const FCONST: u8 = 118;
    pub const ADDR: u8 = 120;
    pub const INDIR: u8 = 123;
}

/// The opcodes whose floating constant `ldobj` puts in memory.
const FLOAT_OPS: &[&str] = &[
    "FMOVF", "FADDF", "FSUBF", "FSUBRF", "FMULF", "FDIVF", "FDIVRF", "FCOMF", "FCOMFP", "MOVSS",
    "ADDSS", "SUBSS", "MULSS", "DIVSS", "COMISS", "UCOMISS", "FMOVD", "FADDD", "FSUBD", "FSUBRD",
    "FMULD", "FDIVD", "FDIVRD", "FCOMD", "FCOMDP", "MOVSD", "ADDSD", "SUBSD", "MULSD", "DIVSD",
    "COMISD", "UCOMISD",
];

/// The records that `span` does not encode.
const PSEUDO: &[&str] = &[
    "TEXT", "DATA", "GLOBL", "HISTORY", "END", "NAME", "SIGNAME", "DYNT", "INIT", "MODE",
];

/// The tables of `6l`, ready for the check.
pub struct Checker {
    /// `ycover[a][b]`: class `a` is covered by class `b`.
    cover: Vec<Vec<bool>>,
    /// The pairs of each opcode name.
    pairs: HashMap<&'static str, &'static [t::Pair]>,
    /// The pairs of `ymovtab` of each opcode name.
    movs: HashMap<&'static str, Vec<(u8, u8)>>,
    /// The value of each class name.
    class: HashMap<&'static str, u8>,
}

/// An instruction that `6l` cannot encode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bad {
    /// The function.
    pub function: String,
    /// The instruction as `6c -S` prints it.
    pub text: String,
    /// The class of the source operand.
    pub from: String,
    /// The class of the destination operand.
    pub to: String,
    /// Why: no `ytab`, or no pair.
    pub why: String,
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}

impl Checker {
    /// Make the tables from `optab6.rs`, as `instinit` in `6l/obj.c` does.
    #[must_use]
    pub fn new() -> Self {
        let n = t::CLASSES.len();
        let mut cover = vec![vec![false; n]; n];
        for (i, row) in cover.iter_mut().enumerate() {
            row[i] = true;
        }
        for &(a, b) in t::COVER {
            cover[usize::from(a)][usize::from(b)] = true;
        }
        let ytabs: HashMap<&str, &[t::Pair]> = t::YTABS.iter().copied().collect();
        let pairs = t::OPTAB
            .iter()
            .filter(|o| !o.1.is_empty())
            .filter_map(|o| Some((o.0, *ytabs.get(o.1)?)))
            .collect();
        let mut movs: HashMap<&str, Vec<(u8, u8)>> = HashMap::new();
        for &(op, a, b) in t::YMOVTAB {
            movs.entry(op).or_default().push((a, b));
        }
        let class = t::CLASSES
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((*c, u8::try_from(i).ok()?)))
            .collect();
        Self {
            cover,
            pairs,
            movs,
            class,
        }
    }

    /// The value of the class `name`.
    fn y(&self, name: &str) -> u8 {
        self.class.get(name).copied().unwrap_or(0)
    }

    /// The class of an operand, as `oclass` in `6l/span.c` gives it, in 64-bit mode.
    #[must_use]
    pub fn oclass(&self, a: &Addr) -> u8 {
        let k = a.kind;
        if k >= d::INDIR || a.index != d::NONE {
            if a.index != d::NONE && a.scale == 0 {
                if k == d::ADDR {
                    return match a.index {
                        d::EXTERN | d::STATIC => self.y("Yi32"),
                        d::AUTO | d::PARAM => self.y("Yiauto"),
                        _ => self.y("Yxxx"),
                    };
                }
                return self.y("Ycol");
            }
            return self.y("Ym");
        }
        let name = match k {
            d::AL => "Yal",
            d::AX => "Yax",
            d::DL | d::BL | d::BPB..=d::R15B | d::AH..=d::BH => "Yrb",
            d::CL => "Ycl",
            d::CX => "Ycx",
            d::DX | d::BX => "Yrx",
            d::SP..=d::R15 => "Yrl",
            d::F0 => "Yf0",
            _ if (d::F0 + 1..d::F0 + 8).contains(&k) => "Yrf",
            _ if (d::M0..d::M0 + 8).contains(&k) => "Ymr",
            _ if (d::X0..d::X0 + 16).contains(&k) => "Yxr",
            d::NONE => "Ynone",
            d::CS..=d::TASK => {
                const SEG: [&str; 11] = [
                    "Ycs", "Yss", "Yds", "Yes", "Yfs", "Ygs", "Ygdtr", "Yidtr", "Yldtr", "Ymsw",
                    "Ytask",
                ];
                SEG[usize::from(k - d::CS)]
            }
            _ if (d::CR..=d::CR + 8).contains(&k) => return self.y(&format!("Ycr{}", k - d::CR)),
            _ if (d::DR..d::DR + 8).contains(&k) => return self.y(&format!("Ydr{}", k - d::DR)),
            _ if (d::TR..d::TR + 8).contains(&k) => return self.y(&format!("Ytr{}", k - d::TR)),
            d::EXTERN | d::STATIC | d::AUTO | d::PARAM => "Ym",
            d::CONST | d::ADDR => {
                if a.sym.is_some() {
                    "Yi32"
                } else {
                    let v = a.offset;
                    match v {
                        0 => "Yi0",
                        1 => "Yi1",
                        -128..=127 => "Yi8",
                        _ if i32::try_from(v).is_ok() => "Ys32",
                        _ if v >> 32 == 0 => "Yi32",
                        _ => "Yi64",
                    }
                }
            }
            d::BRANCH => "Ybr",
            _ => "Yxxx",
        };
        self.y(name)
    }

    /// The changes that `ldobj` and `patch` make to an instruction before `span`.
    fn as_linked(name: &str, p: &Prog) -> Prog {
        let mut p = p.clone();
        if p.from.kind == d::FCONST && FLOAT_OPS.contains(&name) {
            p.from.kind = d::EXTERN;
            p.from.offset = 0;
            p.from.fconst = None;
            p.from.sym = Some(obj::Sym {
                name: "$float".into(),
                kind: d::EXTERN,
            });
        }
        if matches!(name, "CALL" | "RET" | "JMPF") && p.to.sym.is_some() {
            p.to.kind = d::BRANCH;
        }
        p
    }

    /// Check one instruction. Return the classes and why it fails, or `None` when `6l` can
    /// encode it.
    #[must_use]
    pub fn check(&self, name: &str, p: &Prog) -> Option<(u8, u8, String)> {
        if PSEUDO.contains(&name) {
            return None;
        }
        let mut p = Self::as_linked(name, p);
        let mut name = name;
        if name == "RET" && p.from.kind == d::CONST {
            // `dostkoff` makes this tail call an `ADJSP` and a `JMP` to the target.
            name = "JMP";
            p.from = Addr {
                kind: d::NONE,
                index: d::NONE,
                ..Addr::default()
            };
        }
        let (ft, tt) = (self.oclass(&p.from), self.oclass(&p.to));
        let covers = |a: u8, b: u8| {
            self.cover[usize::from(ft)][usize::from(a)]
                && self.cover[usize::from(tt)][usize::from(b)]
        };
        let Some(pairs) = self.pairs.get(name) else {
            return Some((ft, tt, "the opcode has no ytab".into()));
        };
        if pairs.iter().any(|&(a, b, _, _)| covers(a, b)) {
            return None;
        }
        if self
            .movs
            .get(name)
            .is_some_and(|m| m.iter().any(|&(a, b)| covers(a, b)))
        {
            return None;
        }
        Some((
            ft,
            tt,
            "no pair of its ytab or of ymovtab covers the classes".into(),
        ))
    }

    /// Check each instruction of an object.
    #[must_use]
    pub fn object(&self, o: &Object) -> Vec<Bad> {
        let mut out = Vec::new();
        let mut function = String::from("(before any TEXT)");
        let mut text_pc = 0;
        for p in &o.progs {
            let name = o.arch.name(p.op);
            if name == "TEXT" {
                function = p
                    .from
                    .sym
                    .as_ref()
                    .map_or_else(|| "?".into(), |s| s.name.clone());
                text_pc = p.pc.unwrap_or(0);
                continue;
            }
            if let Some((ft, tt, why)) = self.check(&name, p) {
                out.push(Bad {
                    function: function.clone(),
                    text: o.prog(p, text_pc),
                    from: t::CLASSES[usize::from(ft)].into(),
                    to: t::CLASSES[usize::from(tt)].into(),
                    why,
                });
            }
        }
        out
    }
}

/// Check a file: a `.6` object, or an archive of them. Return the report and the number of
/// instructions that fail.
pub fn file(c: &Checker, name: &str, bytes: &[u8]) -> Result<(String, usize), String> {
    let mut objects = Vec::new();
    if let Some(members) = obj::archive(bytes)? {
        for m in members {
            if Arch::of_name(&m.name) != Some(Arch::Amd64) {
                continue;
            }
            let o = obj::decode(Arch::Amd64, &m.data)
                .map_err(|e| format!("{name}({}): {e}", m.name))?;
            objects.push((format!("{name}({})", m.name), o));
        }
    } else {
        if Arch::of_name(name) != Some(Arch::Amd64) {
            return Err(format!(
                "{name}: give a .6 file or an archive of them; arm64 comes at P4"
            ));
        }
        objects.push((
            name.to_string(),
            obj::decode(Arch::Amd64, bytes).map_err(|e| format!("{name}: {e}"))?,
        ));
    }
    let mut s = String::new();
    let mut bad = 0;
    let mut instructions = 0;
    for (n, o) in &objects {
        instructions += o.progs.len();
        for b in c.object(o) {
            bad += 1;
            let _ = writeln!(
                s,
                "{n}: {}: {}: {} and {}: {}",
                b.function,
                b.text.replace('\t', " "),
                b.from,
                b.to,
                b.why
            );
        }
    }
    let _ = writeln!(
        s,
        "{name}: {} objects, {instructions} records, {bad} that 6l cannot encode",
        objects.len()
    );
    Ok((s, bad))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(k: u8) -> Addr {
        Addr {
            kind: k,
            index: d::NONE,
            ..Addr::default()
        }
    }

    fn konst(v: i64) -> Addr {
        Addr {
            offset: v,
            ..reg(d::CONST)
        }
    }

    fn prog(from: Addr, to: Addr) -> Prog {
        Prog {
            op: 0,
            line: 0,
            pc: None,
            reg: 0,
            nosched: false,
            from,
            from3: None,
            to,
        }
    }

    #[test]
    fn operands_get_the_classes_of_6l() {
        let c = Checker::new();
        let name = |a: &Addr| t::CLASSES[usize::from(c.oclass(a))];
        assert_eq!(name(&reg(d::AX)), "Yax");
        assert_eq!(name(&reg(d::SP)), "Yrl");
        assert_eq!(name(&reg(d::X0 + 3)), "Yxr");
        assert_eq!(name(&konst(0)), "Yi0");
        assert_eq!(name(&konst(-5)), "Yi8");
        assert_eq!(name(&konst(100_000)), "Ys32");
        assert_eq!(name(&konst(0xffff_ffff)), "Yi32");
        assert_eq!(name(&konst(1 << 40)), "Yi64");
        assert_eq!(name(&reg(d::INDIR + d::SP)), "Ym");
        assert_eq!(name(&reg(4)), "Yxxx", "SPB has no class, as in 6l");
    }

    #[test]
    fn common_instructions_pass_and_bad_ones_fail() {
        let c = Checker::new();
        assert!(c.check("MOVQ", &prog(konst(1), reg(d::AX))).is_none());
        assert!(c.check("MOVQ", &prog(konst(1 << 40), reg(d::AX))).is_none());
        assert!(c.check("ADDQ", &prog(konst(8), reg(d::SP))).is_none());
        let call = prog(
            reg(d::NONE),
            Addr {
                sym: Some(obj::Sym {
                    name: "print".into(),
                    kind: d::EXTERN,
                }),
                ..reg(d::EXTERN)
            },
        );
        assert!(c.check("CALL", &call).is_none());
        assert!(c.check("RET", &prog(reg(d::NONE), reg(d::NONE))).is_none());
        let mut tail = call.clone();
        tail.from = konst(0);
        assert!(c.check("RET", &tail).is_none(), "a tail call is a JMP");
        // A store of a 64-bit constant to memory has no encoding.
        let bad = c.check("MOVQ", &prog(konst(1 << 40), reg(d::INDIR + d::AX)));
        assert!(bad.is_some());
        // An immediate cannot be the destination.
        assert!(c.check("ADDQ", &prog(reg(d::AX), konst(1))).is_some());
    }
}
