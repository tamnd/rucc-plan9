//! `r9 optab --row arm64`: read the instruction table of `7l` from a pinned tree (plan 07.3).
//!
//! `7l` finds the encoding of an instruction in `optab` of `optab.c`. Each entry gives an opcode,
//! the classes of its three operands, the case of `asmout` that encodes it and the size. The
//! classes are the `C_` values of `l.h`. `buildop` in `span.c` sorts the table, and gives some
//! opcodes the entries of another opcode: `oprange[AADDS] = t` gives `ADDS` the entries of `ADD`.
//! `oplook` takes the first entry whose classes cover the classes of the operands. When there is
//! none, the link stops with "illegal combination".
//!
//! This module parses the three parts, and writes them as Rust. The function `cmp` of `span.c`,
//! which says which classes each class covers, is code and not a table, so rucc has it by hand.

use crate::optab::{braced, entries, strip_comments, words};
use std::fmt::Write as _;
use std::path::Path;

/// One entry of `optab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The opcode, without the `A`.
    pub name: String,
    /// The class of the source operand.
    pub a1: String,
    /// The class of the middle register.
    pub a2: String,
    /// The class of the destination operand.
    pub a3: String,
    /// The case of `asmout` that encodes the entry.
    pub kind: u8,
    /// The size of the encoding in bytes.
    pub size: u8,
}

/// The tables of `7l`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tables {
    /// The `C_` classes, from `C_NONE` to `C_GOK`, in the order of their values.
    pub classes: Vec<String>,
    /// The entries of `optab`, without the last entry `AXXX`.
    pub ops: Vec<Entry>,
    /// The ranges of `buildop`: an opcode and the opcode whose entries it uses.
    pub ranges: Vec<(String, String)>,
}

/// The opcode name without the `A`.
fn op_name(w: &str) -> Result<String, String> {
    w.strip_prefix('A')
        .map(String::from)
        .ok_or_else(|| format!("`{w}` is not an opcode"))
}

/// Parse the `C_` classes from `l.h`, from `C_NONE` to the class before `C_NCLASS`.
pub fn parse_classes(l_h: &str) -> Result<Vec<String>, String> {
    let s = strip_comments(l_h);
    let start = s.find("C_NONE").ok_or("l.h has no C_NONE")?;
    let end = s[start..].find("C_NCLASS").ok_or("l.h has no C_NCLASS")? + start;
    let mut out = Vec::new();
    for w in words(&s[start..end]) {
        let (name, value) = w
            .split_once('=')
            .map_or((w.as_str(), None), |(n, v)| (n.trim(), Some(v.trim())));
        if value.is_some_and(|v| v != "0" || !out.is_empty()) {
            return Err(format!("l.h gives the class {name} a value"));
        }
        if !name.starts_with("C_") {
            return Err(format!("l.h has `{name}` in the classes"));
        }
        out.push(name.to_string());
    }
    Ok(out)
}

/// Parse `optab` from `optab.c`.
pub fn parse_optab(optab_c: &str) -> Result<Vec<Entry>, String> {
    let s = strip_comments(optab_c);
    let start = s
        .find("Optab\toptab[]")
        .or_else(|| s.find("Optab optab[]"))
        .ok_or("optab.c has no optab")?;
    let (body, _) = braced(&s, start)?;
    let mut out = Vec::new();
    for e in entries(body)? {
        if e.len() < 6 {
            return Err(format!("optab: the entry {e:?} is not complete"));
        }
        let name = op_name(&e[0])?;
        if name == "XXX" {
            break;
        }
        // A size can be a product, such as `4*4`.
        let number = |w: &str| {
            w.split('*').try_fold(1_u8, |n, f| {
                let f = f
                    .trim()
                    .parse::<u8>()
                    .map_err(|err| format!("optab: {name}: `{w}`: {err}"))?;
                n.checked_mul(f)
                    .ok_or_else(|| format!("optab: {name}: `{w}` is too large"))
            })
        };
        out.push(Entry {
            a1: e[1].clone(),
            a2: e[2].clone(),
            a3: e[3].clone(),
            kind: number(&e[4])?,
            size: number(&e[5])?,
            name,
        });
    }
    Ok(out)
}

/// Parse the ranges that the `switch` of `buildop` in `span.c` gives. Each `oprange[AY] = t;`
/// gives `Y` the entries of each `case` label before it, up to the `break`.
pub fn parse_ranges(span_c: &str) -> Result<Vec<(String, String)>, String> {
    let s = strip_comments(span_c);
    let start = s.find("buildop(void)").ok_or("span.c has no buildop")?;
    let switch = s[start..]
        .find("switch(r)")
        .ok_or("buildop has no switch(r)")?
        + start;
    let (body, _) = braced(&s, switch)?;
    let mut out = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    for statement in body.split(';') {
        let mut rest = statement.trim();
        loop {
            if let Some(r) = rest.strip_prefix("case") {
                let (label, after) = r
                    .split_once(':')
                    .ok_or_else(|| format!("buildop: `{rest}` has no colon"))?;
                labels.push(op_name(label.trim())?);
                rest = after.trim();
            } else if let Some(r) = rest.strip_prefix("default") {
                rest = r.trim_start().strip_prefix(':').unwrap_or(r).trim();
            } else {
                break;
            }
        }
        if rest.is_empty() {
            continue;
        }
        if rest == "break" {
            labels.clear();
        } else if let Some(r) = rest.strip_prefix("oprange[") {
            let (target, value) = r
                .split_once(']')
                .ok_or_else(|| format!("buildop: `{rest}` is not complete"))?;
            if value.trim() != "= t" {
                return Err(format!("buildop: `{rest}` does not set t"));
            }
            let target = op_name(target.trim())?;
            for label in &labels {
                if *label != target {
                    out.push((target.clone(), label.clone()));
                }
            }
        } else if !rest.starts_with("diag(") && !rest.starts_with("errorexit(") {
            return Err(format!("buildop: `{rest}` is not known"));
        }
    }
    if out.is_empty() {
        return Err("buildop has no ranges".into());
    }
    Ok(out)
}

/// Read the tables of `7l` under `root`, which has `sys/src/cmd/7l`.
pub fn load(root: &Path) -> Result<Tables, String> {
    let dir = root.join("sys/src/cmd/7l");
    let read = |n: &str| {
        let p = dir.join(n);
        std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let ops = parse_optab(&read("optab.c")?)?;
    // `buildop` runs the switch only for an opcode with entries, so a `case` of an opcode with
    // none, such as `ATST`, gives no range.
    let mut ranges = parse_ranges(&read("span.c")?)?;
    ranges.retain(|(_, base)| ops.iter().any(|o| o.name == *base));
    let t = Tables {
        classes: parse_classes(&read("l.h")?)?,
        ops,
        ranges,
    };
    t.validate()?;
    Ok(t)
}

impl Tables {
    /// Check that each class and each opcode of a range is known.
    fn validate(&self) -> Result<(), String> {
        let class = |c: &str| self.classes.iter().any(|x| x == c);
        for o in &self.ops {
            for c in [&o.a1, &o.a2, &o.a3] {
                if !class(c) {
                    return Err(format!("optab: {}: the class {c} is not known", o.name));
                }
            }
        }
        for (op, base) in &self.ranges {
            if !self.ops.iter().any(|o| o.name == *base) {
                return Err(format!("buildop: {op} uses {base}, which has no entry"));
            }
        }
        Ok(())
    }

    fn class(&self, c: &str) -> usize {
        self.classes.iter().position(|x| x == c).unwrap_or(0)
    }

    /// The tables as Rust. The classes are numbers, and `CLASSES` gives their names.
    #[must_use]
    pub fn rust(&self, pin: &str) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "//! The instruction table of `7l` at the pin {pin}: the `C_` classes of `l.h`, `optab` of\n//! `optab.c` and the ranges of `buildop` in `span.c`. `r9 optab --row arm64` writes this file.\n//! Do not edit it.\n"
        );
        let _ = writeln!(
            s,
            "/// The names of the `C_` classes, in the order of their values."
        );
        let _ = writeln!(s, "pub const CLASSES: &[&str] = &[");
        for c in &self.classes {
            let _ = writeln!(s, "    {c:?},");
        }
        let _ = writeln!(s, "];\n");
        let _ = writeln!(
            s,
            "/// The entries of `optab`: the opcode, the classes of the source, the middle register and the\n/// destination, the case of `asmout` and the size."
        );
        let _ = writeln!(s, "pub const OPTAB: &[(&str, u8, u8, u8, u8, u8)] = &[");
        for o in &self.ops {
            let _ = writeln!(
                s,
                "    ({:?}, {}, {}, {}, {}, {}),",
                o.name,
                self.class(&o.a1),
                self.class(&o.a2),
                self.class(&o.a3),
                o.kind,
                o.size
            );
        }
        let _ = writeln!(s, "];\n");
        let _ = writeln!(
            s,
            "/// The ranges of `buildop`: an opcode and the opcode whose entries it uses."
        );
        let _ = writeln!(s, "pub const RANGES: &[(&str, &str)] = &[");
        for (op, base) in &self.ranges {
            let _ = writeln!(s, "    ({op:?}, {base:?}),");
        }
        let _ = writeln!(s, "];");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tables_are_parsed() {
        let l_h = "enum\n{\n\tC_NONE\t\t= 0,\n\tC_REG,\n\tC_RSP,\t\t/* D_REG or D_SP */\n\tC_ZCON,\n\tC_GOK,\n\tC_NCLASS,\t/* must be last */\n};";
        let classes = parse_classes(l_h).unwrap();
        assert_eq!(classes, ["C_NONE", "C_REG", "C_RSP", "C_ZCON", "C_GOK"]);
        let optab_c = "Optab\toptab[] =\n{\n\t{ AADD,\t\tC_REG,\tC_REG,\tC_REG,\t\t 1, 4, 0 },\n\t/* x */\n\t{ AAND,\tC_ZCON,\tC_NONE,\tC_RSP,\t28, 8, 0,\tLFROM },\n\t{ ACASE,\tC_REG,\tC_NONE,\tC_REG,\t62, 4*4, 0 },\n\t{ AXXX,\t\tC_NONE,\tC_NONE,\tC_NONE,\t\t 0, 4, 0 },\n};";
        let ops = parse_optab(optab_c).unwrap();
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[2].size, 16);
        assert_eq!(
            (ops[1].name.as_str(), ops[1].kind, ops[1].size),
            ("AND", 28, 8)
        );
        let span_c = "void\nbuildop(void)\n{\n\tfor(;;) { x; }\n\tswitch(r){\n\tdefault:\n\t\tdiag(\"unknown op in build: %A\", r);\n\t\terrorexit();\n\tcase AADD:\n\t\toprange[AADDS] = t;\n\t\toprange[ASUB] = t;\n\t\tbreak;\n\tcase AAND:\t/* logical */\n\tcase ABIC:\n\t\toprange[AEOR] = t;\n\t\tbreak;\n\tcase AXXX:\n\t\tbreak;\n\t}\n}";
        let ranges = parse_ranges(span_c).unwrap();
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            ranges,
            [
                pair("ADDS", "ADD"),
                pair("SUB", "ADD"),
                pair("EOR", "AND"),
                pair("EOR", "BIC")
            ]
        );
        let tables = Tables {
            classes,
            ops,
            ranges: vec![pair("ADDS", "ADD")],
        };
        tables.validate().unwrap();
        let r = tables.rust("abc");
        assert!(r.contains("    (\"AND\", 3, 0, 2, 28, 8),\n"), "{r}");
        assert!(r.contains("    (\"ADDS\", \"ADD\"),\n"), "{r}");
    }

    #[test]
    fn a_range_of_an_opcode_with_no_entry_is_refused() {
        let tables = Tables {
            classes: vec!["C_NONE".into()],
            ops: Vec::new(),
            ranges: vec![("ADDS".into(), "ADD".into())],
        };
        assert!(tables.validate().is_err());
    }
}
