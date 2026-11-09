//! Read what 9front publishes: the release feed, the release pages and the image directory.
//!
//! The feed is not valid JSON, because the titles have quotes in them that are not escaped. So
//! this module reads it line by line, as it reads the HTML pages.

use std::collections::BTreeSet;

/// One release announcement in the feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Announcement {
    /// The page, with `https` and no port.
    pub page: String,
    /// The release name, such as `THIS WAS SUPPOSED TO BE FUN`.
    pub title: String,
    /// The date in the page path, `YYYY-MM-DD`.
    pub date: String,
}

/// One line of a release page: an image and its SHA-256.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    /// The file name, such as `9front-11952.amd64.qcow2.gz`.
    pub file: String,
    /// The release number, such as `11952`.
    pub number: String,
    /// The SHA-256.
    pub sha256: String,
}

/// The announcements in the feed, newest first.
pub fn feed(text: &str) -> Result<Vec<Announcement>, String> {
    let mut out: Vec<Announcement> = Vec::new();
    let mut page: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(url) = field(line, "url") {
            page = Some(normal_page(url)?);
        } else if let Some(title) = field(line, "title") {
            let Some(page) = page.take() else { continue };
            let title = title
                .trim_start_matches("9FRONT")
                .trim_end_matches("RELEASED")
                .trim()
                .trim_matches('"')
                .to_string();
            let date = page_date(&page)?;
            out.push(Announcement { page, title, date });
        }
    }
    if out.is_empty() {
        return Err("the release feed has no items".into());
    }
    out.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| b.page.cmp(&a.page)));
    Ok(out)
}

/// The value of a line such as `"url": "http://…",`.
fn field<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(&format!("\"{name}\":"))?.trim();
    let rest = rest.strip_suffix(',').unwrap_or(rest).trim();
    rest.strip_prefix('"')?.strip_suffix('"')
}

/// `http://9front.org:80/releases/…` as `https://9front.org/releases/…`.
fn normal_page(url: &str) -> Result<String, String> {
    let path = url
        .strip_prefix("http://9front.org:80")
        .or_else(|| url.strip_prefix("http://9front.org"))
        .or_else(|| url.strip_prefix("https://9front.org"))
        .ok_or_else(|| format!("the feed has a page that is not on 9front.org: {url}"))?;
    Ok(format!("https://9front.org{path}"))
}

/// The date in a page path `…/releases/YYYY/MM/DD/n/`.
fn page_date(page: &str) -> Result<String, String> {
    let parts: Vec<&str> = page
        .split("/releases/")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .collect();
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    match parts.as_slice() {
        [y, m, d, ..] if digits(y, 4) && digits(m, 2) && digits(d, 2) => Ok(format!("{y}-{m}-{d}")),
        _ => Err(format!("{page} has no date in its path")),
    }
}

/// The images and hashes on a release page.
#[must_use]
pub fn page(html: &str) -> Vec<Listed> {
    let text = strip_tags(html);
    let mut out = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let (Some(sha), Some(file), None) = (words.next(), words.next(), words.next()) else {
            continue;
        };
        if !crate::pins::is_hex(sha, 64) {
            continue;
        }
        let Some(number) = number(file) else { continue };
        out.push(Listed {
            file: file.to_string(),
            number,
            sha256: sha.to_string(),
        });
    }
    out
}

/// The release number of `9front-NNNNN.….gz`.
#[must_use]
pub fn number(file: &str) -> Option<String> {
    let rest = file.strip_prefix("9front-")?;
    let (number, kind) = rest.split_once('.')?;
    let ok = !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) && is_gz(kind);
    ok.then(|| number.to_string())
}

/// Whether a file name ends in `.gz`. 9front writes its file names in lower case.
fn is_gz(file: &str) -> bool {
    std::path::Path::new(file)
        .extension()
        .is_some_and(|e| e == "gz")
}

/// The `.gz` files linked from a directory listing.
#[must_use]
pub fn listing(html: &str) -> BTreeSet<String> {
    html.split("href=\"")
        .skip(1)
        .filter_map(|s| s.split('"').next())
        .filter(|f| is_gz(f) && !f.contains('/'))
        .map(str::to_string)
        .collect()
}

/// HTML with its tags taken out. Text between tags stays on its lines.
fn strip_tags(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) const FEED: &str = r#"{
"version":       "https://jsonfeed.org/version/1",
"items": [
        {
  "id": "http://9front.org:80/releases/2026/01/24/0/",
  "url": "http://9front.org:80/releases/2026/01/24/0/",
  "title": "9FRONT "GEFS SERVICE PACK 1" RELEASED",
  "content_html": ""
  }
,
        {
  "id": "http://9front.org:80/releases/2026/08/02/0/",
  "url": "http://9front.org:80/releases/2026/08/02/0/",
  "title": "9FRONT "THIS WAS SUPPOSED TO BE FUN" RELEASED",
  "content_html": ""
  }
]
}"#;

    #[test]
    fn the_feed_is_read_newest_first() {
        let items = feed(FEED).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "THIS WAS SUPPOSED TO BE FUN");
        assert_eq!(items[0].date, "2026-08-02");
        assert_eq!(items[0].page, "https://9front.org/releases/2026/08/02/0/");
        assert_eq!(items[1].title, "GEFS SERVICE PACK 1");
        assert!(feed("{}").is_err());
    }

    #[test]
    fn a_page_gives_its_hashes() {
        let html = "<p>http://9front.org/iso/9front-11554.386.iso.gz.torrent</p>\n<pre>\n   e507559b1b9576e555ccfb3ede24d5cbbfe0c595b9090592cf576c6ae49914f5      9front-11554.386.iso.gz\n   5aaf54327b4bb73a17e192488dc3e65d9d8e526728732e2fdf402bccb8c60236      <a href=x>9front-11554.amd64.iso.gz</a>\n   nothex      9front-11554.pi.img.gz\n</pre>";
        let listed = page(html);
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[1].file, "9front-11554.amd64.iso.gz");
        assert_eq!(listed[1].number, "11554");
    }

    #[test]
    fn a_listing_gives_its_files() {
        let html = r#"<a href="../">..</a><a href="9front-11952.386.iso.gz">x</a><a href="9front-11952.386.iso.gz.torrent">y</a><a href="old/">old</a>"#;
        let files = listing(html);
        assert_eq!(files.len(), 1);
        assert!(files.contains("9front-11952.386.iso.gz"));
        assert_eq!(
            number("9front-11952.amd64.qcow2.gz").as_deref(),
            Some("11952")
        );
        assert_eq!(number("9front-x.amd64.qcow2.gz"), None);
    }
}
