use breezyshim::Tree;
use lazy_regex::regex_is_match;
use std::path::PathBuf;

#[derive(Debug)]
pub struct OddVersion(String);

impl std::fmt::Display for OddVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Odd version: {}", self.0)
    }
}

impl std::error::Error for OddVersion {}

pub struct NewsFile {
    tree: Box<dyn Tree>,
    path: PathBuf,
}

pub fn check_date(d: &str) -> bool {
    if d == "UNRELEASED" || d.starts_with("NEXT ") {
        true
    } else {
        false
    }
}

pub fn check_version(v: &str) -> Result<bool, OddVersion> {
    if v == "UNRELEASED" || v == "%(version)s" || v == "NEXT" {
        return Ok(true);
    }

    if !regex_is_match!(r"^[0-9\.]+$", v) {
        return Err(OddVersion(v.to_string()));
    }

    Ok(false)
}

pub fn skip_header(lines: &[&[u8]]) -> Option<usize> {
    let mut i: usize = 0;
    for (idx, line) in lines.iter().enumerate() {
        if line.starts_with(b"Changelog for ") {
            continue;
        }
        if line.ends_with(b" release notes") {
            continue;
        }
        if line.iter().all(|&x| x == b'=' || x == b'-') {
            continue;
        }
        if line.is_empty() {
            continue;
        }
        i = idx;
        break;
    }
    if i == 0 && !lines.is_empty() && lines[0].is_empty() {
        None
    } else {
        Some(i)
    }
}
