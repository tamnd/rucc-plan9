//! The objects of `6c` and `7c`, and `r9 objdiff` (plan 07.6).
//!
//! An object is a list of records with no header. A record starts with its opcode in two bytes.
//! `NAME` and `SIGNAME` bind a symbol to a slot from 1 to 49, and the operands that follow give
//! the slot. Every other record is a `Prog` with a line number and two operands, and with a third
//! operand and a register on arm64. The format is the one that `ldobj` and `zaddr` read in
//! `6l/obj.c` and `7l/obj.c`. A file can hold more than one object, each ended by `END`.
//!
//! The decoder resolves each slot to its symbol, so that a listing does not depend on the order
//! in which a compiler gives slots. The listing prints each `Prog` as `6c -S` and `7c -S` do,
//! from `Pconv` and `Dconv` in `6c/list.c` and `7c/list.c`. A branch target is printed as the
//! number of instructions from the `TEXT` of its function.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::anames;

/// The two architectures whose objects `r9` reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// `.6` files of `6c` and `6a`.
    Amd64,
    /// `.7` files of `7c` and `7a`.
    Arm64,
}

impl Arch {
    /// The architecture of a file name: `.6` or `.7`.
    #[must_use]
    pub fn of_name(name: &str) -> Option<Self> {
        if name.ends_with(".6") {
            Some(Self::Amd64)
        } else if name.ends_with(".7") {
            Some(Self::Arm64)
        } else {
            None
        }
    }

    fn names(self) -> &'static [&'static str] {
        match self {
            Self::Amd64 => anames::AMD64,
            Self::Arm64 => anames::ARM64,
        }
    }

    /// The number of the opcode `name`. Every name used here is in both tables.
    fn op(self, name: &str) -> u16 {
        let n = self.names().iter().position(|n| *n == name);
        u16::try_from(n.unwrap_or(usize::MAX)).unwrap_or(u16::MAX)
    }

    /// The name of opcode `op`.
    #[must_use]
    pub fn name(self, op: u16) -> String {
        self.names()
            .get(usize::from(op))
            .map_or_else(|| format!("GOK({op})"), |n| (*n).to_string())
    }
}

/// The operand types of amd64 that the decoder needs, from `6.out.h`.
pub(crate) mod d6 {
    pub const NONE: u8 = 111;
    pub const BRANCH: u8 = 112;
    pub const EXTERN: u8 = 113;
    pub const STATIC: u8 = 114;
    pub const AUTO: u8 = 115;
    pub const PARAM: u8 = 116;
    pub const CONST: u8 = 117;
    pub const FCONST: u8 = 118;
    pub const SCONST: u8 = 119;
    pub const ADDR: u8 = 120;
    pub const INDIR: u8 = 123;
    pub const T_TYPE: u8 = 1 << 0;
    pub const T_INDEX: u8 = 1 << 1;
    pub const T_OFFSET: u8 = 1 << 2;
    pub const T_FCONST: u8 = 1 << 3;
    pub const T_SYM: u8 = 1 << 4;
    pub const T_SCONST: u8 = 1 << 5;
    pub const T_64: u8 = 1 << 6;
}

/// The operand types of arm64 that the decoder needs, from `7.out.h`.
mod d7 {
    pub const NONE: u8 = 1;
    pub const EXTERN: u8 = 2;
    pub const STATIC: u8 = 3;
    pub const AUTO: u8 = 4;
    pub const PARAM: u8 = 5;
    pub const BRANCH: u8 = 6;
    pub const OREG: u8 = 7;
    pub const XPRE: u8 = 8;
    pub const XPOST: u8 = 9;
    pub const CONST: u8 = 10;
    pub const DCONST: u8 = 11;
    pub const FCONST: u8 = 12;
    pub const SCONST: u8 = 13;
    pub const REG: u8 = 14;
    pub const SP: u8 = 15;
    pub const FREG: u8 = 16;
    pub const VREG: u8 = 17;
    pub const SPR: u8 = 18;
    pub const SHIFT: u8 = 22;
    pub const EXTREG: u8 = 27;
    pub const ROFF: u8 = 28;
    pub const COND: u8 = 29;
    pub const NREG: u8 = 32;
    const fn sysarg5(op0: u32, op1: u32, cn: u32, cm: u32, op2: u32) -> u32 {
        op0 << 19 | op1 << 16 | cn << 12 | cm << 8 | op2 << 5
    }
    pub const DAIF: u32 = sysarg5(3, 3, 4, 2, 1);
    pub const NZCV: u32 = sysarg5(3, 3, 4, 2, 0);
    pub const FPSR: u32 = sysarg5(3, 3, 4, 4, 1);
    pub const FPCR: u32 = sysarg5(3, 3, 4, 4, 0);
}

/// A symbol that an operand names, with the type of its `NAME` record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sym {
    /// The name.
    pub name: String,
    /// The type of the `NAME` record: extern, static, auto, param or file.
    pub kind: u8,
}

/// One operand. The fields that an architecture does not use stay at their zero values.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Addr {
    /// The operand type.
    pub kind: u8,
    /// The index register (amd64), or `NONE`.
    pub index: u8,
    /// The scale (amd64). For `TEXT`, `DATA` and `GLOBL` it holds the flags or the width.
    pub scale: u8,
    /// The register (arm64), or `NREG`.
    pub reg: u8,
    /// The name class of the symbol (arm64): extern, static, auto, param or none.
    pub name: u8,
    /// The offset or the integer constant.
    pub offset: i64,
    /// The symbol.
    pub sym: Option<Sym>,
    /// The bits of a floating constant.
    pub fconst: Option<u64>,
    /// The bytes of a string constant.
    pub sconst: Option<[u8; 8]>,
}

/// One record that is not a `NAME`.
#[derive(Debug, Clone, PartialEq)]
pub struct Prog {
    /// The opcode.
    pub op: u16,
    /// The line, in the history of the object.
    pub line: u32,
    /// The pc from the start of the object, for the records that take one.
    pub pc: Option<u32>,
    /// The middle register (arm64), or `NREG`. For `TEXT` and `DATA` it holds the flags or the width.
    pub reg: u8,
    /// The no-schedule mark (arm64).
    pub nosched: bool,
    /// The source operand.
    pub from: Addr,
    /// The third operand (arm64).
    pub from3: Option<Addr>,
    /// The destination operand.
    pub to: Addr,
}

/// A decoded object.
#[derive(Debug, Clone, PartialEq)]
pub struct Object {
    /// The architecture.
    pub arch: Arch,
    /// The records, without the `NAME` records.
    pub progs: Vec<Prog>,
    /// The signature of each symbol that a `SIGNAME` record gives.
    pub signatures: BTreeMap<String, u32>,
}

/// A reader over the bytes of an object.
struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn u8(&mut self) -> Result<u8, String> {
        let v = *self
            .b
            .get(self.at)
            .ok_or_else(|| format!("the object ends at byte {}", self.at))?;
        self.at += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from(self.u8()?) | u16::from(self.u8()?) << 8)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from(self.u16()?) | u32::from(self.u16()?) << 16)
    }

    fn bytes8(&mut self) -> Result<[u8; 8], String> {
        let mut v = [0u8; 8];
        for x in &mut v {
            *x = self.u8()?;
        }
        Ok(v)
    }

    fn cstr(&mut self) -> Result<String, String> {
        let rest = &self.b[self.at..];
        let n = rest
            .iter()
            .position(|&c| c == 0)
            .ok_or("a name has no end")?;
        self.at += n + 1;
        Ok(String::from_utf8_lossy(&rest[..n]).into_owned())
    }
}

/// The symbol in `slot`.
fn slot(h: &[Option<Sym>; 50], s: u8) -> Result<Option<Sym>, String> {
    h.get(usize::from(s))
        .cloned()
        .ok_or_else(|| format!("symbol slot {s} is out of range"))
}

/// Read an operand of amd64, as `zaddr` in `6l/obj.c`.
fn addr6(r: &mut Reader, h: &[Option<Sym>; 50]) -> Result<Addr, String> {
    let t = r.u8()?;
    let mut a = Addr {
        kind: d6::NONE,
        index: d6::NONE,
        ..Addr::default()
    };
    if t & d6::T_INDEX != 0 {
        a.index = r.u8()?;
        a.scale = r.u8()?;
    }
    if t & d6::T_OFFSET != 0 {
        let lo = r.u32()?;
        a.offset = i64::from(lo.cast_signed());
        if t & d6::T_64 != 0 {
            let hi = r.u32()?;
            a.offset = (i64::from(hi) << 32) | i64::from(lo);
        }
    }
    if t & d6::T_SYM != 0 {
        a.sym = slot(h, r.u8()?)?;
    }
    if t & d6::T_FCONST != 0 {
        let lo = r.u32()?;
        let hi = r.u32()?;
        a.fconst = Some(u64::from(hi) << 32 | u64::from(lo));
        a.kind = d6::FCONST;
    } else if t & d6::T_SCONST != 0 {
        a.sconst = Some(r.bytes8()?);
        a.kind = d6::SCONST;
    }
    if t & d6::T_TYPE != 0 {
        a.kind = r.u8()?;
    }
    Ok(a)
}

/// Read an operand of arm64, as `zaddr` in `7l/obj.c`.
fn addr7(r: &mut Reader, h: &[Option<Sym>; 50]) -> Result<Addr, String> {
    let kind = r.u8()?;
    let reg = r.u8()?;
    let sym = slot(h, r.u8()?)?;
    let name = r.u8()?;
    let mut a = Addr {
        kind,
        reg,
        name,
        sym,
        ..Addr::default()
    };
    match kind {
        d7::OREG
        | d7::XPRE
        | d7::XPOST
        | d7::CONST
        | d7::BRANCH
        | d7::SHIFT
        | d7::EXTREG
        | d7::ROFF
        | d7::SPR => a.offset = i64::from(r.u32()?.cast_signed()),
        d7::DCONST => {
            let lo = r.u32()?;
            let hi = r.u32()?;
            a.offset = (i64::from(hi) << 32) | i64::from(lo);
            a.kind = d7::CONST;
        }
        d7::SCONST => a.sconst = Some(r.bytes8()?),
        d7::FCONST => {
            let lo = r.u32()?;
            let hi = r.u32()?;
            a.fconst = Some(u64::from(hi) << 32 | u64::from(lo));
        }
        _ => {}
    }
    Ok(a)
}

/// True when `op` takes a pc in the linker. `NAME`, `HISTORY`, `END`, `GLOBL`, `DATA`, `DYNT`,
/// `INIT` and `MODE` take none.
fn takes_pc(arch: Arch, op: u16) -> bool {
    !["HISTORY", "END", "GLOBL", "DATA", "DYNT", "INIT", "MODE"]
        .iter()
        .any(|n| arch.op(n) == op)
}

/// Decode an object.
pub fn decode(arch: Arch, bytes: &[u8]) -> Result<Object, String> {
    let (name_op, signame_op, end_op, last) = (
        arch.op("NAME"),
        arch.op("SIGNAME"),
        arch.op("END"),
        arch.op("LAST"),
    );
    let mut r = Reader { b: bytes, at: 0 };
    let mut h: [Option<Sym>; 50] = std::array::from_fn(|_| None);
    let mut obj = Object {
        arch,
        progs: Vec::new(),
        signatures: BTreeMap::new(),
    };
    let mut pc = 0u32;
    while r.at < bytes.len() {
        let start = r.at;
        let op = r.u16()?;
        if op == 0 || op >= last {
            return Err(format!("opcode {op} at byte {start} is out of range"));
        }
        if op == name_op || op == signame_op {
            let sig = if op == signame_op {
                Some(r.u32()?)
            } else {
                None
            };
            let kind = r.u8()?;
            let s = r.u8()?;
            let name = r.cstr()?;
            if let Some(sig) = sig {
                obj.signatures.insert(name.clone(), sig);
            }
            *h.get_mut(usize::from(s))
                .ok_or_else(|| format!("symbol slot {s} is out of range"))? =
                Some(Sym { name, kind });
            continue;
        }
        let p = match arch {
            Arch::Amd64 => {
                let line = r.u32()?;
                let from = addr6(&mut r, &h)?;
                let to = addr6(&mut r, &h)?;
                Prog {
                    op,
                    line,
                    pc: None,
                    reg: 0,
                    nosched: false,
                    from,
                    from3: None,
                    to,
                }
            }
            Arch::Arm64 => {
                let b = r.u8()?;
                let line = r.u32()?;
                let from = addr7(&mut r, &h)?;
                let from3 = if b & 0x40 == 0 {
                    None
                } else {
                    Some(addr7(&mut r, &h)?)
                };
                let to = addr7(&mut r, &h)?;
                Prog {
                    op,
                    line,
                    pc: None,
                    reg: b & 0x3F,
                    nosched: b & 0x80 != 0,
                    from,
                    from3,
                    to,
                }
            }
        };
        let mut p = p;
        if takes_pc(arch, op) {
            p.pc = Some(pc);
            pc += 1;
        }
        let end = op == end_op;
        obj.progs.push(p);
        if end {
            h = std::array::from_fn(|_| None);
            pc = 0;
        }
    }
    Ok(obj)
}

/// The text of a string constant, as `Sconv` writes it.
fn sconst(s: [u8; 8]) -> String {
    let mut o = String::new();
    for c in s {
        match c {
            0 => o.push_str("\\z"),
            b'\\' | b'"' => {
                o.push('\\');
                o.push(char::from(c));
            }
            b'\n' => o.push_str("\\n"),
            b'\t' => o.push_str("\\t"),
            b'\r' => o.push_str("\\r"),
            b' '..=b'~' => o.push(char::from(c)),
            _ => {
                let _ = write!(o, "\\{c:o}");
            }
        }
    }
    o
}

/// The name of an amd64 register, as `Rconv` in `6c/list.c` writes it.
#[must_use]
pub fn reg6(r: u8) -> String {
    const B: [&str; 16] = [
        "AL", "CL", "DL", "BL", "SPB", "BPB", "SIB", "DIB", "R8B", "R9B", "R10B", "R11B", "R12B",
        "R13B", "R14B", "R15B",
    ];
    const W: [&str; 16] = [
        "AX", "CX", "DX", "BX", "SP", "BP", "SI", "DI", "R8", "R9", "R10", "R11", "R12", "R13",
        "R14", "R15",
    ];
    const OTHER: [&str; 11] = [
        "CS", "SS", "DS", "ES", "FS", "GS", "GDTR", "IDTR", "LDTR", "MSW", "TASK",
    ];
    match r {
        0..=15 => B[usize::from(r)].into(),
        16..=31 => W[usize::from(r - 16)].into(),
        32..=35 => ["AH", "CH", "DH", "BH"][usize::from(r - 32)].into(),
        36..=43 => format!("F{}", r - 36),
        44..=51 => format!("M{}", r - 44),
        52..=67 => format!("X{}", r - 52),
        68..=78 => OTHER[usize::from(r - 68)].into(),
        79..=94 => format!("CR{}", r - 79),
        95..=102 => format!("DR{}", r - 95),
        103..=110 => format!("TR{}", r - 103),
        111 => "NONE".into(),
        _ => format!("gok({r})"),
    }
}

/// The name of a symbol, or `?` when the slot is empty.
fn symname(a: &Addr) -> &str {
    a.sym.as_ref().map_or("?", |s| s.name.as_str())
}

/// An amd64 operand, as `Dconv` in `6c/list.c` writes it. `text` is the pc of the `TEXT` of the
/// function, for branch targets.
fn dconv6(a: &Addr, text: u32) -> String {
    let k = a.kind;
    let o = a.offset;
    let s = if k >= d6::INDIR {
        if o == 0 {
            format!("({})", reg6(k - d6::INDIR))
        } else {
            format!("{o}({})", reg6(k - d6::INDIR))
        }
    } else {
        match k {
            d6::NONE => String::new(),
            d6::BRANCH => format!("{}(PC)", o - i64::from(text)),
            d6::EXTERN => format!("{}+{o}(SB)", symname(a)),
            d6::STATIC => format!("{}<>+{o}(SB)", symname(a)),
            d6::AUTO => format!("{}+{o}(SP)", symname(a)),
            d6::PARAM if a.sym.is_some() => format!("{}+{o}(FP)", symname(a)),
            d6::PARAM => format!("{o}(FP)"),
            d6::CONST => format!("${o}"),
            d6::FCONST => format!("$({:.17e})", f64::from_bits(a.fconst.unwrap_or(0))),
            d6::SCONST => format!("$\"{}\"", sconst(a.sconst.unwrap_or_default())),
            d6::ADDR => {
                let inner = Addr {
                    kind: a.index,
                    index: d6::NONE,
                    ..a.clone()
                };
                return format!("${}", dconv6(&inner, text));
            }
            _ if o != 0 => format!("${o},{}", reg6(k)),
            _ => reg6(k),
        }
    };
    if a.index == d6::NONE {
        s
    } else {
        format!("{s}({}*{})", reg6(a.index), a.scale)
    }
}

/// The offset of an arm64 operand with its symbol, as `Nconv` in `7c/list.c` writes it.
fn nconv7(a: &Addr) -> String {
    let o = a.offset;
    let Some(s) = &a.sym else {
        return format!("{o}");
    };
    match a.name {
        d7::NONE => format!("{o}"),
        d7::EXTERN => format!("{}+{o}(SB)", s.name),
        d7::STATIC => format!("{}<>+{o}(SB)", s.name),
        d7::AUTO => format!("{}-{}(SP)", s.name, -o),
        d7::PARAM => format!("{}+{o}(FP)", s.name),
        n => format!("GOK-name({n})"),
    }
}

/// An arm64 operand, as `Dconv` in `7c/list.c` writes it. The kinds that `7c` does not print get
/// a plain form.
fn dconv7(a: &Addr, text: u32) -> String {
    const EXTOP: [&str; 8] = [".UB", ".UH", ".UW", ".UX", ".SB", ".SH", ".SW", ".SX"];
    let o = a.offset;
    let named = a.name != d7::NONE || a.sym.is_some();
    let reg = |r: u8| {
        if r == d7::NREG {
            String::new()
        } else {
            format!("(R{r})")
        }
    };
    match a.kind {
        d7::NONE if named || a.reg != d7::NREG => format!("{}(R{})(NONE)", nconv7(a), a.reg),
        d7::NONE => String::new(),
        d7::CONST => format!("${}{}", nconv7(a), reg(a.reg)),
        d7::SHIFT => {
            let v = o;
            let op = ["<<", ">>", "->", "@>"][usize::try_from((v >> 5) & 3).unwrap_or(0)];
            let s = if v & (1 << 4) != 0 {
                format!("R{}{op}R{}", v & 15, (v >> 8) & 15)
            } else {
                format!("R{}{op}{}", v & 15, (v >> 7) & 31)
            };
            format!("{s}{}", reg(a.reg))
        }
        d7::OREG => format!("{}{}", nconv7(a), reg(a.reg)),
        d7::XPRE => format!("{}{}!", nconv7(a), reg(a.reg)),
        d7::XPOST if a.reg != d7::NREG => format!("(R{}){}!", a.reg, nconv7(a)),
        d7::XPOST => format!("{}!", nconv7(a)),
        d7::EXTREG => {
            let ext = EXTOP[usize::try_from((o >> 13) & 7).unwrap_or(0)];
            if o & (7 << 10) != 0 {
                format!("R{}{ext}<<{}", (o >> 16) & 31, (o >> 10) & 7)
            } else {
                format!("R{}{ext}", (o >> 16) & 31)
            }
        }
        d7::REG | d7::SP | d7::FREG | d7::VREG if named => {
            format!("{}(R{})(REG)", nconv7(a), a.reg)
        }
        d7::REG => format!("R{}", a.reg),
        d7::SP => "SP".into(),
        d7::FREG => format!("F{}", a.reg),
        d7::VREG => format!("V{}", a.reg),
        d7::SPR => match u32::try_from(o).unwrap_or(0) {
            d7::FPSR => "FPSR".into(),
            d7::FPCR => "FPCR".into(),
            d7::NZCV => "NZCV".into(),
            d7::DAIF => "DAIF".into(),
            v => format!("SPR({v:#x})"),
        },
        d7::BRANCH => format!("{}(PC)", o - i64::from(text)),
        d7::FCONST => format!("${:.17e}", f64::from_bits(a.fconst.unwrap_or(0))),
        d7::SCONST => format!("$\"{}\"", sconst(a.sconst.unwrap_or_default())),
        d7::ROFF => format!("ROFF({o:#x}){}", reg(a.reg)),
        d7::COND => format!("COND({})", a.reg),
        k => format!("GOK-type({k})"),
    }
}

impl Object {
    /// The text of one `Prog`, as `Pconv` writes it, with a tab before the opcode and none
    /// before the operands. `text` is the pc of the `TEXT` of its function.
    #[must_use]
    pub fn prog(&self, p: &Prog, text: u32) -> String {
        let name = self.arch.name(p.op);
        match self.arch {
            Arch::Amd64 => {
                let (from, to) = (dconv6(&p.from, text), dconv6(&p.to, text));
                if name == "DATA" {
                    format!("{name}\t{from}/{},{to}", p.from.scale)
                } else if name == "TEXT" {
                    format!("{name}\t{from},{},{to}", p.from.scale)
                } else {
                    format!("{name}\t{from},{to}")
                }
            }
            Arch::Arm64 => {
                let mut s = format!("{name}\t{}", dconv7(&p.from, text));
                if name == "DATA" {
                    let _ = write!(s, "/{}", p.reg);
                } else if name == "TEXT" {
                    let _ = write!(s, ",{}", p.reg);
                } else if p.reg != d7::NREG {
                    let f = matches!(p.from.kind, d7::FREG | d7::FCONST);
                    let _ = write!(s, ",{}{}", if f { 'F' } else { 'R' }, p.reg);
                }
                if let Some(f3) = &p.from3
                    && f3.kind != d7::NONE
                {
                    let _ = write!(s, ",{}", dconv7(f3, text));
                }
                if p.to.kind != d7::NONE {
                    let sep = if s.ends_with('\t') { "" } else { "," };
                    let _ = write!(s, "{sep}{}", dconv7(&p.to, text));
                }
                s.trim_end_matches('\t').to_string()
            }
        }
    }

    /// The listing of the object, grouped by symbol. A key is `TEXT name` for a function, with
    /// its instructions, or `DATA name` for the `GLOBL` and `DATA` records of a symbol. A static
    /// symbol has `<>` after its name. The `HISTORY` and `END` records are left out.
    #[must_use]
    pub fn symbols(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let (text_op, data_op, globl_op) = (
            self.arch.op("TEXT"),
            self.arch.op("DATA"),
            self.arch.op("GLOBL"),
        );
        let skip = [self.arch.op("HISTORY"), self.arch.op("END")];
        let mut current: Option<(String, u32)> = None;
        let mut data: BTreeMap<String, Vec<DataLine>> = BTreeMap::new();
        for p in &self.progs {
            if skip.contains(&p.op) {
                continue;
            }
            let key_of = |p: &Prog| {
                let s = symname(&p.from).to_string();
                let stat = p.from.sym.as_ref().is_some_and(|s| {
                    s.kind
                        == match self.arch {
                            Arch::Amd64 => d6::STATIC,
                            Arch::Arm64 => d7::STATIC,
                        }
                });
                if stat { format!("{s}<>") } else { s }
            };
            if p.op == text_op {
                let key = format!("TEXT {}", key_of(p));
                let pc = p.pc.unwrap_or(0);
                out.entry(key.clone()).or_default().push(self.prog(p, pc));
                current = Some((key, pc));
            } else if p.op == data_op || p.op == globl_op {
                let key = format!("DATA {}", key_of(p));
                let order = (p.op != globl_op, p.from.offset);
                data.entry(key).or_default().push((order, self.prog(p, 0)));
            } else if let Some((key, pc)) = &current {
                let line = self.prog(p, *pc);
                out.entry(key.clone()).or_default().push(line);
            } else {
                out.entry("(before any TEXT)".into())
                    .or_default()
                    .push(self.prog(p, 0));
            }
        }
        // The DATA records of a symbol can come in any order, so they are sorted by offset,
        // after the GLOBL.
        for (k, mut v) in data {
            v.sort();
            out.insert(k, v.into_iter().map(|(_, l)| l).collect());
        }
        out
    }
}

/// A `GLOBL` or `DATA` line with its place: the `GLOBL` first, then the `DATA` by offset.
type DataLine = ((bool, i64), String);

/// One member of an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    /// The name.
    pub name: String,
    /// The bytes.
    pub data: Vec<u8>,
}

/// The magic of an archive.
const ARMAG: &[u8] = b"!<arch>\n";

/// The members of an archive, or `None` when `bytes` is not an archive. `__.SYMDEF` is left
/// out, because `ar` writes it from the other members.
pub fn archive(bytes: &[u8]) -> Result<Option<Vec<Member>>, String> {
    if !bytes.starts_with(ARMAG) {
        return Ok(None);
    }
    let mut at = ARMAG.len();
    let mut out = Vec::new();
    while at + 60 <= bytes.len() {
        let hdr = &bytes[at..at + 60];
        if &hdr[58..60] != b"`\n" {
            return Err(format!("the archive header at byte {at} is not valid"));
        }
        let name = String::from_utf8_lossy(&hdr[..16])
            .trim_end()
            .trim_end_matches('/')
            .to_string();
        let size: usize = String::from_utf8_lossy(&hdr[48..58])
            .trim()
            .parse()
            .map_err(|e| format!("the archive member at byte {at}: size: {e}"))?;
        let start = at + 60;
        let data = bytes
            .get(start..start + size)
            .ok_or_else(|| format!("the archive member {name} ends after the file"))?;
        if name != "__.SYMDEF" {
            out.push(Member {
                name,
                data: data.to_vec(),
            });
        }
        at = start + size + (size & 1);
    }
    Ok(Some(out))
}

/// The edit of `left` into `right` from the longest common subsequence: `' '` for a line that
/// both have, `-` for a line that only `left` has and `+` for a line that only `right` has. A
/// `-` comes before a `+` where the order is free. `None` when the table would be too large.
#[must_use]
pub fn edit_ops<'a>(left: &'a [String], right: &'a [String]) -> Option<Vec<(char, &'a str)>> {
    let (nl, nr) = (left.len(), right.len());
    if nl.saturating_mul(nr) > 16_000_000 {
        return None;
    }
    let mut lcs = vec![vec![0u32; nr + 1]; nl + 1];
    for i in (0..nl).rev() {
        for j in (0..nr).rev() {
            lcs[i][j] = if left[i] == right[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut ops: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < nl || j < nr {
        if i < nl && j < nr && left[i] == right[j] {
            ops.push((' ', &left[i]));
            i += 1;
            j += 1;
        } else if i < nl && (j == nr || lcs[i + 1][j] >= lcs[i][j + 1]) {
            ops.push(('-', &left[i]));
            i += 1;
        } else {
            ops.push(('+', &right[j]));
            j += 1;
        }
    }
    Some(ops)
}

/// The lines of a listing that differ, as a short diff from the longest common subsequence.
/// Lines that only `a` has start with `-`, and lines that only `b` has start with `+`. At most
/// `context` same lines are kept around each change.
#[must_use]
pub fn line_diff(left: &[String], right: &[String], context: usize) -> Vec<String> {
    let Some(ops) = edit_ops(left, right) else {
        return vec![
            format!("- {} lines", left.len()),
            format!("+ {} lines", right.len()),
        ];
    };
    let near: BTreeSet<usize> = ops
        .iter()
        .enumerate()
        .filter(|(_, (c, _))| *c != ' ')
        .flat_map(|(k, _)| k.saturating_sub(context)..=k + context)
        .collect();
    let mut out = Vec::new();
    let mut last = None;
    for (k, (c, line)) in ops.iter().enumerate() {
        if !near.contains(&k) {
            continue;
        }
        if last.is_some_and(|x: usize| x + 1 != k) {
            out.push("...".into());
        }
        out.push(format!("{c} {line}"));
        last = Some(k);
    }
    out
}

/// Compare two decoded objects, symbol by symbol. The result is empty when the listings and the
/// signatures are the same.
#[must_use]
pub fn diff_objects(a: &Object, b: &Object, context: usize) -> String {
    let mut rep = String::new();
    let (sa, sb) = (a.symbols(), b.symbols());
    for (key, la) in &sa {
        match sb.get(key) {
            None => {
                let _ = writeln!(rep, "only in A: {key}");
            }
            Some(lb) if la != lb => {
                let _ = writeln!(rep, "differ: {key}");
                for line in line_diff(la, lb, context) {
                    let _ = writeln!(rep, "  {line}");
                }
            }
            Some(_) => {}
        }
    }
    for key in sb.keys().filter(|key| !sa.contains_key(*key)) {
        let _ = writeln!(rep, "only in B: {key}");
    }
    let names: BTreeSet<&String> = a.signatures.keys().chain(b.signatures.keys()).collect();
    for name in names {
        let (x, y) = (a.signatures.get(name), b.signatures.get(name));
        if x != y {
            let show = |v: Option<&u32>| v.map_or("none".to_string(), |v| format!("{v:#010x}"));
            let _ = writeln!(rep, "signature of {name}: {} and {}", show(x), show(y));
        }
    }
    rep
}

/// Decode a file of `arch`: one object, or the members of an archive that end in `.6` or `.7`.
pub fn objects(name: &str, bytes: &[u8]) -> Result<Vec<(String, Object)>, String> {
    if let Some(members) = archive(bytes)? {
        let mut out = Vec::new();
        for m in members {
            let arch = Arch::of_name(&m.name)
                .ok_or_else(|| format!("{name}({}): not a .6 or .7 member", m.name))?;
            let o = decode(arch, &m.data).map_err(|e| format!("{name}({}): {e}", m.name))?;
            out.push((m.name, o));
        }
        return Ok(out);
    }
    let arch = Arch::of_name(name)
        .ok_or_else(|| format!("{name}: give a .6 or .7 file, or an archive of them"))?;
    Ok(vec![(
        String::new(),
        decode(arch, bytes).map_err(|e| format!("{name}: {e}"))?,
    )])
}

/// The listing of a file, symbol by symbol.
pub fn listing(name: &str, bytes: &[u8]) -> Result<String, String> {
    let mut s = String::new();
    for (member, o) in objects(name, bytes)? {
        if !member.is_empty() {
            let _ = writeln!(s, "member {member}");
        }
        for (k, lines) in o.symbols() {
            let _ = writeln!(s, "{k}");
            for l in lines {
                let _ = writeln!(s, "\t{l}");
            }
        }
        for (n, sig) in &o.signatures {
            let _ = writeln!(s, "signature {n} {sig:#010x}");
        }
    }
    Ok(s)
}

/// Compare two files: two objects, or two archives member by member. Return the report and
/// whether the files are the same in their records. Line numbers are not compared.
pub fn diff_files(
    an: &str,
    a: &[u8],
    bn: &str,
    b: &[u8],
    context: usize,
) -> Result<(String, bool), String> {
    if a == b {
        return Ok(("the files are the same, byte for byte\name".into(), true));
    }
    let (oa, ob) = (objects(an, a)?, objects(bn, b)?);
    let ma: BTreeMap<&str, &Object> = oa.iter().map(|(name, o)| (name.as_str(), o)).collect();
    let mb: BTreeMap<&str, &Object> = ob.iter().map(|(name, o)| (name.as_str(), o)).collect();
    let mut rep = String::new();
    for (name, ox) in &ma {
        match mb.get(name) {
            None => {
                let _ = writeln!(rep, "member only in A: {name}");
            }
            Some(oy) => {
                let d = diff_objects(ox, oy, context);
                if !d.is_empty() {
                    if !name.is_empty() {
                        let _ = writeln!(rep, "member {name}");
                    }
                    rep.push_str(&d);
                }
            }
        }
    }
    for name in mb.keys().filter(|name| !ma.contains_key(*name)) {
        let _ = writeln!(rep, "member only in B: {name}");
    }
    let same = rep.is_empty();
    if same {
        rep.push_str("the records are the same; the files differ only in line numbers or in the order of names\name");
    }
    Ok((rep, same))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An amd64 object: `TEXT main(SB),0,$8`, `MOVQ $1,AX`, `JMP` back to the `MOVQ`, `RET`,
    /// `END`, with `main` in slot 1.
    pub(crate) fn amd64_object() -> Vec<u8> {
        amd64_object_to(16)
    }

    /// The object of `amd64_object`, with the operand type `to` as the destination of `MOVQ`.
    pub(crate) fn amd64_object_to(to: u8) -> Vec<u8> {
        let a = Arch::Amd64;
        let op = |n: &str| a.op(n).to_le_bytes();
        let mut b = Vec::new();
        b.extend(op("SIGNAME"));
        b.extend(0x1234_5678u32.to_le_bytes());
        b.extend([d6::EXTERN, 1]);
        b.extend(b"main\0");
        // TEXT main(SB), 0, $8
        b.extend(op("TEXT"));
        b.extend(7u32.to_le_bytes());
        b.extend([
            d6::T_INDEX | d6::T_SYM | d6::T_TYPE,
            d6::NONE,
            0,
            1,
            d6::EXTERN,
        ]);
        b.extend([d6::T_OFFSET | d6::T_TYPE]);
        b.extend(8u32.to_le_bytes());
        b.push(d6::CONST);
        // MOVQ $1, AX
        b.extend(op("MOVQ"));
        b.extend(8u32.to_le_bytes());
        b.extend([d6::T_OFFSET | d6::T_TYPE]);
        b.extend(1u32.to_le_bytes());
        b.push(d6::CONST);
        b.extend([d6::T_TYPE, to]);
        // JMP 1(PC), back to the MOVQ
        b.extend(op("JMP"));
        b.extend(9u32.to_le_bytes());
        b.push(0);
        b.extend([d6::T_OFFSET | d6::T_TYPE]);
        b.extend(1u32.to_le_bytes());
        b.push(d6::BRANCH);
        // RET
        b.extend(op("RET"));
        b.extend(10u32.to_le_bytes());
        b.extend([0, 0]);
        b.extend(op("END"));
        b.extend(10u32.to_le_bytes());
        b.extend([0, 0]);
        b
    }

    #[test]
    fn an_amd64_object_is_listed_as_6c_does() {
        let o = decode(Arch::Amd64, &amd64_object()).unwrap();
        assert_eq!(o.signatures["main"], 0x1234_5678);
        let s = o.symbols();
        assert_eq!(
            s["TEXT main"],
            [
                "TEXT\tmain+0(SB),0,$8",
                "MOVQ\t$1,AX",
                "JMP\t,1(PC)",
                "RET\t,"
            ]
        );
    }

    #[test]
    fn line_numbers_do_not_make_a_difference() {
        let a = amd64_object();
        let mut b = a.clone();
        // The SIGNAME record takes 13 bytes, so the line of the TEXT starts at byte 15.
        assert_eq!(b[15], 7);
        b[15] = 99;
        let (r, same) = diff_files("a.6", &a, "b.6", &b, 2).unwrap();
        assert!(same, "{r}");
        let mut c = a.clone();
        let at = c.windows(4).position(|w| w == 1u32.to_le_bytes()).unwrap();
        c[at] = 2;
        let (r, same) = diff_files("a.6", &a, "c.6", &c, 2).unwrap();
        assert!(!same);
        assert!(r.contains("differ: TEXT main\n"), "{r}");
        assert!(r.contains("- MOVQ\t$1,AX\n"), "{r}");
        assert!(r.contains("+ MOVQ\t$2,AX\n"), "{r}");
    }

    #[test]
    fn an_arm64_operand_is_printed_as_7c_does() {
        let a = Addr {
            kind: d7::OREG,
            reg: 1,
            name: d7::NONE,
            offset: 16,
            ..Addr::default()
        };
        assert_eq!(dconv7(&a, 0), "16(R1)");
        let s = Addr {
            kind: d7::OREG,
            reg: d7::NREG,
            name: d7::AUTO,
            offset: -8,
            sym: Some(Sym {
                name: "x".into(),
                kind: d7::AUTO,
            }),
            ..Addr::default()
        };
        assert_eq!(dconv7(&s, 0), "x-8(SP)");
        let r = Addr {
            kind: d7::REG,
            reg: 3,
            name: d7::NONE,
            ..Addr::default()
        };
        assert_eq!(dconv7(&r, 0), "R3");
    }

    #[test]
    fn amd64_registers_have_their_names() {
        assert_eq!(reg6(0), "AL");
        assert_eq!(reg6(16), "AX");
        assert_eq!(reg6(31), "R15");
        assert_eq!(reg6(52), "X0");
        assert_eq!(reg6(67), "X15");
        assert_eq!(reg6(79), "CR0");
        assert_eq!(reg6(111), "NONE");
    }

    #[test]
    fn an_archive_gives_its_members() {
        let mut b = ARMAG.to_vec();
        for (n, d) in [("__.SYMDEF", &b"xyz"[..]), ("a.6", &b"ab"[..])] {
            b.extend(
                format!(
                    "{n:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
                    0,
                    0,
                    0,
                    644,
                    d.len()
                )
                .bytes(),
            );
            b.extend(d);
            if d.len() % 2 == 1 {
                b.push(b'\n');
            }
        }
        let m = archive(&b).unwrap().unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(
            (m[0].name.as_str(), m[0].data.as_slice()),
            ("a.6", &b"ab"[..])
        );
        assert!(archive(b"not").unwrap().is_none());
    }

    #[test]
    fn a_line_diff_keeps_context() {
        let a: Vec<String> = ["a", "b", "c", "d", "e"].map(String::from).to_vec();
        let b: Vec<String> = ["a", "b", "x", "d", "e"].map(String::from).to_vec();
        assert_eq!(line_diff(&a, &b, 1), ["  b", "- c", "+ x", "  d"]);
    }
}
