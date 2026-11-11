//! `r9 mix`: build a tree with some units from R and the others from G (plan 05.7, 12.8).
//!
//! A mix gives each unit a side. A unit is a source file of the guest, such as
//! `/sys/src/libc/port/strcmp.c`, or a directory, which applies to each file under it. The
//! longest name that matches a file gives its side, and a file that no name matches gets the
//! default side.
//!
//! `r9 ccd` reads the mix. When a call has a source on side G, `r9 ccd` does not run rucc, and
//! replies with the status `mix: G`. Under `cc:auto`, `r9-cc` then runs kencc for the call. Thus
//! a call that compiles two files is R only when both files are R.
//!
//! A mix is given as a list of `UNIT=R` and `UNIT=G` items, with commas between them, or as a
//! file with one item on each line. In the file, a line that starts with `#` is a comment.

use std::collections::BTreeMap;
use std::fmt::{self, Write as _};

/// The reply status of a call that the mix gives to G.
pub const STATUS_G: &str = "mix: G";

/// The compiler of a unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// rucc.
    R,
    /// kencc.
    G,
}

impl Side {
    /// Parse `R` or `G`, in either case.
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim() {
            "R" | "r" => Ok(Self::R),
            "G" | "g" => Ok(Self::G),
            other => Err(format!("mix: `{other}` is not R or G")),
        }
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::R => "R",
            Self::G => "G",
        })
    }
}

/// The side of each unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mix {
    /// The side of a file that no unit matches.
    pub default: Side,
    /// The side of each unit, by its name with no `/` at the end.
    pub units: BTreeMap<String, Side>,
}

impl Mix {
    /// A mix with no unit.
    #[must_use]
    pub const fn new(default: Side) -> Self {
        Self {
            default,
            units: BTreeMap::new(),
        }
    }

    /// Add one item, `UNIT=R` or `UNIT=G`. A unit must be an absolute path of the guest.
    pub fn add(&mut self, item: &str) -> Result<(), String> {
        let (unit, side) = item
            .rsplit_once('=')
            .ok_or_else(|| format!("mix: `{item}` is not UNIT=R or UNIT=G"))?;
        let unit = unit.trim();
        if !unit.starts_with('/') {
            return Err(format!("mix: the unit `{unit}` is not an absolute path"));
        }
        let unit = crate::ccd::absolute("/", unit);
        self.units.insert(unit, Side::parse(side)?);
        Ok(())
    }

    /// Add each item of a list with commas between the items.
    pub fn add_list(&mut self, list: &str) -> Result<(), String> {
        list.split(',')
            .filter(|s| !s.trim().is_empty())
            .try_for_each(|item| self.add(item))
    }

    /// Add each item of the text of a mix file.
    pub fn add_file(&mut self, text: &str) -> Result<(), String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .try_for_each(|item| self.add(item))
    }

    /// The side of the file `path`, an absolute path of the guest.
    #[must_use]
    pub fn side(&self, path: &str) -> Side {
        let mut name = path;
        loop {
            if let Some(side) = self.units.get(name) {
                return *side;
            }
            match name.rfind('/') {
                Some(0) | None => return self.default,
                Some(i) => name = &name[..i],
            }
        }
    }

    /// The side of a call that compiles `sources`: R when each source is R.
    #[must_use]
    pub fn side_of_call<'a>(&self, sources: impl IntoIterator<Item = &'a str>) -> Side {
        if sources.into_iter().all(|s| self.side(s) == Side::R) {
            Side::R
        } else {
            Side::G
        }
    }

    /// The text of the mix as a mix file.
    #[must_use]
    pub fn text(&self) -> String {
        let mut out = format!("# default {}\n", self.default);
        for (unit, side) in &self.units {
            let _ = writeln!(out, "{unit}={side}");
        }
        out
    }
}

/// The mix of the options `--default`, `--file` and `--mix` of `args`. Return `None` when no
/// option gives a mix.
pub fn from_args(args: &crate::cli::Args) -> Result<Option<Mix>, String> {
    if args.get("default").is_none() && args.get("file").is_none() && args.get("mix").is_none() {
        return Ok(None);
    }
    let default = args.get("default").map_or(Ok(Side::R), Side::parse)?;
    let mut mix = Mix::new(default);
    if let Some(path) = args.get("mix") {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{path}: {e}"))?;
        mix.add_file(&text).map_err(|e| format!("{path}: {e}"))?;
    }
    if let Some(list) = args.get("file") {
        mix.add_list(list)?;
    }
    Ok(Some(mix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_longest_unit_gives_the_side() {
        let mut mix = Mix::new(Side::G);
        mix.add_list("/sys/src/libc=R, /sys/src/libc/port/strcmp.c=G,/sys/src/cmd/acme/=r")
            .unwrap();
        assert_eq!(mix.side("/sys/src/libc/port/strcmp.c"), Side::G);
        assert_eq!(mix.side("/sys/src/libc/port/strlen.c"), Side::R);
        assert_eq!(mix.side("/sys/src/libcomplete/x.c"), Side::G);
        assert_eq!(mix.side("/sys/src/cmd/acme/acme.c"), Side::R);
        assert_eq!(mix.side("/sys/src/cmd/sam/sam.c"), Side::G);
        assert_eq!(
            mix.side_of_call(["/sys/src/libc/port/strlen.c", "/sys/src/libc/port/strcmp.c"]),
            Side::G
        );
        assert_eq!(mix.side_of_call(["/sys/src/libc/port/strlen.c"]), Side::R);
    }

    #[test]
    fn a_mix_file_has_comments_and_round_trips() {
        let mut mix = Mix::new(Side::R);
        mix.add_file("# the units of acme\n\n/sys/src/cmd/acme/exec.c=G\n")
            .unwrap();
        assert_eq!(mix.text(), "# default R\n/sys/src/cmd/acme/exec.c=G\n");
        let mut again = Mix::new(Side::R);
        again.add_file(&mix.text()).unwrap();
        assert_eq!(again, mix);
    }

    #[test]
    fn a_bad_item_is_refused() {
        let mut mix = Mix::new(Side::R);
        assert!(mix.add("acme.c=G").unwrap_err().contains("absolute"));
        assert!(mix.add("/sys/src/acme.c").unwrap_err().contains("UNIT=R"));
        assert!(
            mix.add("/sys/src/acme.c=K")
                .unwrap_err()
                .contains("not R or G")
        );
    }
}
