//! `r9 sets`: expand `sets.toml` into `pins.toml` (plan 04.1).

use crate::repo::Repo;
use r9_pins::expand::{Expanded, expand};
use r9_pins::pins::{FILE_NAME, PinsFile};
use r9_pins::sets::Sets;
use r9_pins::web::Web;
use std::fmt::Write as _;

/// What `r9 sets` found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// The report to print.
    pub report: String,
    /// Whether `pins.toml` already named these trees and images.
    pub up_to_date: bool,
}

/// Run `r9 sets`. `as_of` is a UTC time, or `None` for now.
pub fn run(
    repo: &Repo,
    web: &dyn Web,
    as_of: Option<&str>,
    write: bool,
) -> Result<Outcome, String> {
    if let Some(t) = as_of {
        check_time(t)?;
    }
    let sets = Sets::load(&repo.root)?;
    let old = PinsFile::load(&repo.root)?;
    let time = as_of.map_or_else(now, str::to_string);
    let Expanded { file, notes } = expand(&sets, web, &time, as_of, old.as_ref())?;
    let up_to_date = old.as_ref().is_some_and(|o| same_pins(o, &file));

    let mut report = table(&file);
    for note in &notes {
        let _ = writeln!(report, "note: {note}");
    }
    if write {
        let path = repo.root.join(FILE_NAME);
        std::fs::write(&path, file.render()?).map_err(|e| format!("{}: {e}", path.display()))?;
        let _ = writeln!(report, "wrote {FILE_NAME} as of {time}");
    } else if up_to_date {
        let _ = writeln!(report, "{FILE_NAME} is up to date");
    } else {
        let _ = writeln!(report, "{FILE_NAME} is out of date; run r9 sets --write");
    }
    Ok(Outcome { report, up_to_date })
}

/// Whether two files pin the same things. The time they were taken does not count.
#[must_use]
pub fn same_pins(a: &PinsFile, b: &PinsFile) -> bool {
    a.tree == b.tree && a.release == b.release
}

fn table(file: &PinsFile) -> String {
    let mut out = String::new();
    for (name, tree) in &file.tree {
        let _ = writeln!(
            out,
            "{name:<8} {:<10} {} {}",
            tree.sets.join(","),
            tree.commit.get(..12).unwrap_or(&tree.commit),
            tree.date.get(..10).unwrap_or(&tree.date)
        );
    }
    for (name, release) in file.release.iter().rev() {
        let commit = if release.commit.is_empty() {
            "no commit yet"
        } else {
            release.commit.get(..12).unwrap_or(&release.commit)
        };
        let _ = writeln!(
            out,
            "{name:<8} {:<16} {commit} {} {} images: {}",
            release.sets.join(","),
            release.date,
            release.image.len(),
            release.title
        );
    }
    out
}

/// Refuse a time that is not written as `YYYY-MM-DDTHH:MM:SSZ`.
fn check_time(t: &str) -> Result<(), String> {
    let b = t.as_bytes();
    let digits =
        |r: std::ops::Range<usize>| r.clone().all(|i| b.get(i).is_some_and(u8::is_ascii_digit));
    let ok = b.len() == 20
        && digits(0..4)
        && b[4] == b'-'
        && digits(5..7)
        && b[7] == b'-'
        && digits(8..10)
        && b[10] == b'T'
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19)
        && b[19] == b'Z';
    if ok {
        Ok(())
    } else {
        Err(format!(
            "--as-of {t}: give a UTC time such as 2026-11-09T06:00:00Z"
        ))
    }
}

/// The time now, as `YYYY-MM-DDTHH:MM:SSZ`.
#[must_use]
pub fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    utc(secs)
}

/// Seconds since 1970 as `YYYY-MM-DDTHH:MM:SSZ`, by the civil-from-days method.
#[must_use]
pub fn utc(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0);
    let rest = secs % 86_400;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times() {
        assert_eq!(utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc(1_794_091_301), "2026-11-07T22:41:41Z");
        assert!(check_time("2026-11-09T06:00:00Z").is_ok());
        assert!(check_time("2026-11-09").is_err());
        assert!(check_time("2026-11-09T06:00:00+00:00").is_err());
    }
}
