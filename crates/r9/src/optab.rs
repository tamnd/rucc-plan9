//! `r9 optab`: read the instruction tables of `6l` from a pinned tree (plan 07.3).
//!
//! `6l` finds the encoding of an instruction in two tables. `optab` in `optab.c` gives each
//! opcode a list of operand class pairs (a `ytab`), and `ymovtab` in `span.c` gives more pairs for
//! the moves to and from special registers. The classes are the `Y` values of `l.h`. An
//! instruction whose operand classes are covered by no pair is the error "doasm: notfound".
//!
//! `instinit` in `obj.c` makes `ycover`, which says which classes each class covers. This module
//! parses the four files, and writes the tables as Rust. `optab6.rs` in this crate is
//! the output for the pin, and `r9 objcheck` uses it. The same output goes to rucc, so that the
//! lowering of rucc can be checked against the linker at build time.

use std::fmt::Write as _;
use std::path::Path;

/// One pair of a `ytab`: the classes of the operands, the encoding and its size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pair {
    /// The class of the source operand.
    pub from: String,
    /// The class of the destination operand.
    pub to: String,
    /// The encoding, a `Z` value.
    pub z: String,
    /// The number of opcode bytes that the pair uses.
    pub n: u8,
}

/// One entry of `optab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    /// The opcode, without the `A`.
    pub name: String,
    /// The `ytab`, or empty for `AXXX`.
    pub ytab: String,
    /// The prefix.
    pub prefix: String,
    /// The opcode bytes, as the source writes them.
    pub bytes: Vec<String>,
}

/// One entry of `ymovtab`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mov {
    /// The opcode, without the `A`.
    pub name: String,
    /// The class of the source operand.
    pub from: String,
    /// The class of the destination operand.
    pub to: String,
}

/// A `ytab` array: its name and its pairs.
pub type Ytab = (String, Vec<Pair>);

/// The tables of `6l`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tables {
    /// The `Y` classes, in the order of their values.
    pub classes: Vec<String>,
    /// The `ytab` arrays, in the order of the source.
    pub ytabs: Vec<Ytab>,
    /// The entries of `optab`.
    pub ops: Vec<Op>,
    /// The entries of `ymovtab`.
    pub movs: Vec<Mov>,
    /// The pairs of `ycover` other than a class with itself: a class and a class it covers.
    pub cover: Vec<(String, String)>,
}

/// Remove the comments of C source.
pub(crate) fn strip_comments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix("/*") {
            rest = r.find("*/").map_or("", |i| &r[i + 2..]);
            out.push(' ');
        } else if let Some(r) = rest.strip_prefix("//") {
            rest = r.find('\n').map_or("", |i| &r[i..]);
        } else {
            let c = rest.chars().next().unwrap_or(' ');
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    out
}

/// The text between the brace after `at` and the brace that closes it.
pub(crate) fn braced(s: &str, at: usize) -> Result<(&str, usize), String> {
    let open = s[at..].find('{').ok_or("no opening brace")? + at;
    let mut depth = 0;
    for (i, c) in s[open..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Ok((&s[open + 1..open + i], open + i + 1));
                }
            }
            _ => {}
        }
    }
    Err("no closing brace".into())
}

/// The words of a list that commas separate.
pub(crate) fn words(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|w| !w.is_empty())
        .map(String::from)
        .collect()
}

/// The entries of an array of structures: the words of each inner brace.
pub(crate) fn entries(body: &str) -> Result<Vec<Vec<String>>, String> {
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(i) = body[at..].find('{') {
        let (inner, end) = braced(body, at + i)?;
        out.push(words(inner));
        at = end;
    }
    Ok(out)
}

/// The opcode name without the `A`.
fn op_name(w: &str) -> Result<String, String> {
    w.strip_prefix('A')
        .map(String::from)
        .ok_or_else(|| format!("`{w}` is not an opcode"))
}

/// Parse the `Y` classes from `l.h`.
pub fn parse_classes(l_h: &str) -> Result<Vec<String>, String> {
    let s = strip_comments(l_h);
    let start = s.find("Yxxx").ok_or("l.h has no Yxxx")?;
    let end = s[start..].find("Ymax").ok_or("l.h has no Ymax")? + start;
    let mut out = Vec::new();
    for w in words(&s[start..end]) {
        let (name, value) = w
            .split_once('=')
            .map_or((w.as_str(), None), |(n, v)| (n.trim(), Some(v.trim())));
        if value.is_some_and(|v| v != "0" || !out.is_empty()) {
            return Err(format!("l.h gives the class {name} a value"));
        }
        out.push(name.to_string());
    }
    Ok(out)
}

/// Parse the `ytab` arrays and `optab` from `optab.c`.
pub fn parse_optab(optab_c: &str) -> Result<(Vec<Ytab>, Vec<Op>), String> {
    let s = strip_comments(optab_c);
    let mut ytabs = Vec::new();
    let mut at = 0;
    while let Some(i) = s[at..].find("uchar") {
        let head = at + i + "uchar".len();
        let bracket = s[head..].find('[').ok_or("a uchar array has no [")? + head;
        let name = s[head..bracket].trim().to_string();
        let (body, end) = braced(&s, bracket)?;
        let w = words(body);
        let mut pairs = Vec::new();
        let mut k = 0;
        while k < w.len() && w[k] != "0" {
            let q = w
                .get(k..k + 4)
                .ok_or_else(|| format!("{name}: a pair is not complete"))?;
            pairs.push(Pair {
                from: q[0].clone(),
                to: q[1].clone(),
                z: q[2].clone(),
                n: q[3].parse().map_err(|e| format!("{name}: {e}"))?,
            });
            k += 4;
        }
        ytabs.push((name, pairs));
        at = end;
    }
    let start = s
        .find("Optab optab[]")
        .or_else(|| s.find("Optab\toptab[]"))
        .ok_or("optab.c has no optab")?;
    let (body, _) = braced(&s, start)?;
    let mut ops = Vec::new();
    for e in entries(body)? {
        let Some(first) = e.first() else { continue };
        ops.push(Op {
            name: op_name(first)?,
            ytab: e.get(1).cloned().unwrap_or_default(),
            prefix: e.get(2).cloned().unwrap_or_default(),
            bytes: e.iter().skip(3).cloned().collect(),
        });
    }
    Ok((ytabs, ops))
}

/// Parse `ymovtab` from `span.c`.
pub fn parse_movtab(span_c: &str) -> Result<Vec<Mov>, String> {
    let s = strip_comments(span_c);
    let start = s.find("ymovtab[]").ok_or("span.c has no ymovtab")?;
    let (body, _) = braced(&s, start)?;
    let mut out = Vec::new();
    for e in entries(body)? {
        if e.len() < 3 {
            return Err(format!("ymovtab: the entry {e:?} is not complete"));
        }
        out.push(Mov {
            name: op_name(&e[0])?,
            from: e[1].clone(),
            to: e[2].clone(),
        });
    }
    Ok(out)
}

/// Parse the pairs of `ycover` that `instinit` in `obj.c` sets, such as
/// `ycover[Yi0*Ymax + Yi8] = 1;`.
pub fn parse_cover(obj_c: &str) -> Result<Vec<(String, String)>, String> {
    let s = strip_comments(obj_c);
    let mut out = Vec::new();
    for line in s.lines().map(str::trim) {
        let Some(rest) = line.strip_prefix("ycover[") else {
            continue;
        };
        let Some((index, value)) = rest.split_once(']') else {
            continue;
        };
        if index.starts_with("i*") {
            continue;
        }
        let (a, b) = index
            .split_once("*Ymax")
            .and_then(|(a, b)| Some((a.trim(), b.trim().strip_prefix('+')?.trim())))
            .ok_or_else(|| format!("obj.c: `{line}` is not a ycover pair"))?;
        if value.trim().trim_end_matches(';').trim() != "= 1" {
            return Err(format!("obj.c: `{line}` does not set 1"));
        }
        out.push((a.to_string(), b.to_string()));
    }
    if out.is_empty() {
        return Err("obj.c has no ycover pairs".into());
    }
    Ok(out)
}

/// Read the tables of `6l` under `root`, which has `sys/src/cmd/6l`.
pub fn load(root: &Path) -> Result<Tables, String> {
    let dir = root.join("sys/src/cmd/6l");
    let read = |n: &str| {
        let p = dir.join(n);
        std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))
    };
    let classes = parse_classes(&read("l.h")?)?;
    let (ytabs, ops) = parse_optab(&read("optab.c")?)?;
    let movs = parse_movtab(&read("span.c")?)?;
    let cover = parse_cover(&read("obj.c")?)?;
    let t = Tables {
        classes,
        ytabs,
        ops,
        movs,
        cover,
    };
    t.validate()?;
    Ok(t)
}

impl Tables {
    /// Check that each name in the tables is known.
    fn validate(&self) -> Result<(), String> {
        let class = |c: &str| self.classes.iter().any(|x| x == c);
        for (name, pairs) in &self.ytabs {
            for p in pairs {
                if !class(&p.from) || !class(&p.to) {
                    return Err(format!("{name}: an unknown class in {p:?}"));
                }
            }
        }
        for o in &self.ops {
            if !o.ytab.is_empty() && !self.ytabs.iter().any(|(n, _)| *n == o.ytab) {
                return Err(format!("{}: the ytab {} is not known", o.name, o.ytab));
            }
        }
        for m in &self.movs {
            if !class(&m.from) || !class(&m.to) {
                return Err(format!("ymovtab: an unknown class in {m:?}"));
            }
        }
        for (a, b) in &self.cover {
            if !class(a) || !class(b) {
                return Err(format!("ycover: an unknown class in {a} and {b}"));
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
            "//! The instruction tables of `6l` at the pin {pin}: the `Y` classes of `l.h`, `optab` of\n//! `optab.c` and `ymovtab` of `span.c`. `r9 optab` writes this file. Do not edit it.\n"
        );
        let _ = writeln!(
            s,
            "/// The names of the `Y` classes, in the order of their values."
        );
        let _ = writeln!(s, "pub const CLASSES: &[&str] = &[");
        for c in &self.classes {
            let _ = writeln!(s, "    {c:?},");
        }
        let _ = writeln!(s, "];\n");
        for c in ["Ynone", "Ym", "Ycol", "Ybr", "Ymax"] {
            if let Some(i) = self.classes.iter().position(|x| x == c) {
                let _ = writeln!(
                    s,
                    "/// The class `{c}`.\npub const {}: u8 = {i};\n",
                    c.to_uppercase()
                );
            }
        }
        let _ = writeln!(
            s,
            "/// One pair of a `ytab`: the source class, the destination class, the encoding and its size."
        );
        let _ = writeln!(s, "pub type Pair = (u8, u8, &'static str, u8);\n");
        let _ = writeln!(s, "/// The `ytab` arrays.");
        let _ = writeln!(s, "pub const YTABS: &[(&str, &[Pair])] = &[");
        for (name, pairs) in &self.ytabs {
            let _ = write!(s, "    ({name:?}, &[");
            for (i, p) in pairs.iter().enumerate() {
                let sep = if i == 0 { "" } else { ", " };
                let _ = write!(
                    s,
                    "{sep}({}, {}, {:?}, {})",
                    self.class(&p.from),
                    self.class(&p.to),
                    p.z,
                    p.n
                );
            }
            let _ = writeln!(s, "]),");
        }
        let _ = writeln!(s, "];\n");
        let _ = writeln!(
            s,
            "/// The entries of `optab`: the opcode, its `ytab`, the prefix and the opcode bytes."
        );
        let _ = writeln!(s, "pub const OPTAB: &[(&str, &str, &str, &[&str])] = &[");
        for o in &self.ops {
            let _ = writeln!(
                s,
                "    ({:?}, {:?}, {:?}, &{:?}),",
                o.name, o.ytab, o.prefix, o.bytes
            );
        }
        let _ = writeln!(s, "];\n");
        let _ = writeln!(
            s,
            "/// The entries of `ymovtab`: the opcode, the source class and the destination class."
        );
        let _ = writeln!(s, "pub const YMOVTAB: &[(&str, u8, u8)] = &[");
        for m in &self.movs {
            let _ = writeln!(
                s,
                "    ({:?}, {}, {}),",
                m.name,
                self.class(&m.from),
                self.class(&m.to)
            );
        }
        let _ = writeln!(s, "];");
        let _ = writeln!(
            s,
            "\n/// The pairs of `ycover` other than a class with itself: a class and a class it covers."
        );
        let _ = writeln!(s, "pub const COVER: &[(u8, u8)] = &[");
        for (a, b) in &self.cover {
            let _ = writeln!(s, "    ({}, {}),", self.class(a), self.class(b));
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
        let l_h =
            "enum {\n\tYxxx = 0,\n\tYnone,\n\tYi0, /* zero */\n\tYml,\n\tYmax,\n\tZxxx = 0,\n};";
        assert_eq!(parse_classes(l_h).unwrap(), ["Yxxx", "Ynone", "Yi0", "Yml"]);
        let optab_c = "uchar\tynone[] =\n{\n\tYnone,\tYnone,\tZlit,\t1,\n\t0\n};\nuchar\tyinc[] =\n{\n\tYnone,\tYml,\tZo_m,\t2,\n\t0\n};\nOptab optab[] =\n/* as */\n{\n\t{ AXXX },\n\t{ AAAA,\t\tynone,\tP32, 0x37 },\n\t{ AINCL,\tyinc,\tPx, 0xff,(00) },\n\t0\n};";
        let (ytabs, ops) = parse_optab(optab_c).unwrap();
        assert_eq!(ytabs.len(), 2);
        assert_eq!(ytabs[1].1[0].to, "Yml");
        assert_eq!(ops.len(), 3);
        assert_eq!(ops[0].name, "XXX");
        assert_eq!(ops[2].bytes, ["0xff", "(00)"]);
        let span_c = "Movtab\tymovtab[] =\n{\n/* push */\n\t{APUSHL,\tYnone,\tYml,\t0,\t0x0e,E,0,0},\n\t0\n};";
        let movs = parse_movtab(span_c).unwrap();
        assert_eq!(movs[0].name, "PUSHL");
        let tables = Tables {
            classes: parse_classes(l_h).unwrap(),
            ytabs,
            ops,
            movs,
            cover: parse_cover("\tfor(i=0; i<Ymax; i++)\n\t\tycover[i*Ymax + i] = 1;\n\tycover[Yi0*Ymax + Yml] = 1;\n").unwrap(),
        };
        assert_eq!(tables.cover, [("Yi0".to_string(), "Yml".to_string())]);
        tables.validate().unwrap();
        let r = tables.rust("abc");
        assert!(r.contains("(\"yinc\", &[(1, 3, \"Zo_m\", 2)]),"), "{r}");
        assert!(r.contains("(\"PUSHL\", 1, 3),"), "{r}");
        assert!(
            r.contains("pub const COVER: &[(u8, u8)] = &[\n    (2, 3),\n];"),
            "{r}"
        );
    }
}
