//! `r9 sdiff`: the code of two objects side by side, function by function (plan 12.8).
//!
//! When bisection ends at one unit, the R object and the G object of the unit are the input.
//! For each function, the listing of A is on the left and the listing of B is on the right. The
//! lines are aligned on the longest common subsequence, as `r9 objdiff` aligns them. The column
//! between the two sides is empty for a line that both have, `|` for a line that changed, `<`
//! for a line that only A has and `>` for a line that only B has.

use std::fmt::Write as _;

use crate::obj;

/// The options of one comparison.
#[derive(Debug, Clone, Copy)]
pub struct Options<'a> {
    /// Show only this function.
    pub function: Option<&'a str>,
    /// The width of one side, in characters.
    pub width: usize,
    /// Show the functions that are the same too.
    pub all: bool,
}

/// Cut or pad `s` to `width` characters.
fn cell(s: &str, width: usize) -> String {
    let s = s.replace('\t', " ");
    let n = s.chars().count();
    if n > width {
        let mut t: String = s.chars().take(width.saturating_sub(1)).collect();
        t.push('~');
        t
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

/// Write one row.
fn row(out: &mut String, left: &str, mark: char, right: &str, width: usize) {
    let line = format!("{} {mark} {}", cell(left, width), right.replace('\t', " "));
    let _ = writeln!(out, "{}", line.trim_end());
}

/// The two listings of one function side by side.
#[must_use]
pub fn columns(left: &[String], right: &[String], width: usize) -> String {
    let mut out = String::new();
    let Some(ops) = obj::edit_ops(left, right) else {
        let _ = writeln!(
            out,
            "  ({} lines and {} lines, too many to align)",
            left.len(),
            right.len()
        );
        return out;
    };
    let mut k = 0;
    while k < ops.len() {
        if ops[k].0 == ' ' {
            row(&mut out, ops[k].1, ' ', ops[k].1, width);
            k += 1;
            continue;
        }
        // A run of changes: the lines of A and then the lines of B, paired row by row.
        let start = k;
        while k < ops.len() && ops[k].0 != ' ' {
            k += 1;
        }
        let run = &ops[start..k];
        let olds: Vec<&str> = run.iter().filter(|o| o.0 == '-').map(|o| o.1).collect();
        let news: Vec<&str> = run.iter().filter(|o| o.0 == '+').map(|o| o.1).collect();
        for i in 0..olds.len().max(news.len()) {
            match (olds.get(i), news.get(i)) {
                (Some(a), Some(b)) => row(&mut out, a, '|', b, width),
                (Some(a), None) => row(&mut out, a, '<', "", width),
                (None, Some(b)) => row(&mut out, "", '>', b, width),
                (None, None) => {}
            }
        }
    }
    out
}

/// Compare two files side by side: two objects, or two archives member by member. Return the
/// report and whether a function differs.
pub fn files(
    an: &str,
    a: &[u8],
    bn: &str,
    b: &[u8],
    opt: Options,
) -> Result<(String, bool), String> {
    let left = obj::objects(an, a)?;
    let right = obj::objects(bn, b)?;
    let mut out = String::new();
    let mut differ = false;
    let mut found = false;
    for (member, oa) in &left {
        let Some((_, ob)) = right.iter().find(|(m, _)| m == member) else {
            let _ = writeln!(out, "only in A: member {member}");
            differ = true;
            continue;
        };
        let (sa, sb) = (oa.symbols(), ob.symbols());
        let keys: std::collections::BTreeSet<&String> = sa
            .keys()
            .chain(sb.keys())
            .filter(|k| k.starts_with("TEXT "))
            .collect();
        for key in keys {
            let name = &key["TEXT ".len()..];
            if opt.function.is_some_and(|f| f != name) {
                continue;
            }
            found = true;
            let empty = Vec::new();
            let la = sa.get(key).unwrap_or(&empty);
            let lb = sb.get(key).unwrap_or(&empty);
            if la == lb && !opt.all {
                continue;
            }
            differ |= la != lb;
            let place = if member.is_empty() {
                String::new()
            } else {
                format!(" in {member}")
            };
            let _ = writeln!(
                out,
                "{key}{place}: {} lines in A, {} lines in B{}",
                la.len(),
                lb.len(),
                if la == lb { ", the same" } else { "" }
            );
            out.push_str(&columns(la, lb, opt.width));
            out.push('\n');
        }
    }
    for (member, _) in &right {
        if !left.iter().any(|(m, _)| m == member) {
            let _ = writeln!(out, "only in B: member {member}");
            differ = true;
        }
    }
    if let Some(f) = opt.function
        && !found
    {
        return Err(format!("no function {f} in {an} or {bn}"));
    }
    Ok((out, differ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn the_rows_are_marked_by_what_changed() {
        let a = lines(&["TEXT f", "MOVQ $1,AX", "ADDQ BX,AX", "RET"]);
        let b = lines(&["TEXT f", "MOVL $1,AX", "RET", "NOP"]);
        let got = columns(&a, &b, 12);
        let want = "TEXT f         TEXT f\n\
                    MOVQ $1,AX   | MOVL $1,AX\n\
                    ADDQ BX,AX   <\n\
                    RET            RET\n\
                    \x20            > NOP\n";
        assert_eq!(got, want);
    }

    #[test]
    fn a_long_line_is_cut_to_the_width() {
        let a = lines(&["MOVQ\tverylongname+0(SB),AX"]);
        let got = columns(&a, &[], 10);
        assert_eq!(got, "MOVQ very~ <\n");
    }
}
