use breezyshim::tree::{MutableTree, Tree};
use lazy_regex::regex_is_match;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct OddVersion(String);

impl std::fmt::Display for OddVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Odd version: {}", self.0)
    }
}

impl std::error::Error for OddVersion {}

pub struct NewsFile {
    tree: Box<dyn MutableTree>,
    path: PathBuf,
}

impl NewsFile {
    fn mark_released(&self, expected_version: &str, release_date: &str) {
        news_mark_released(&self.tree, &self.path, expected_version, release_date);
    }

    fn add_pending(&self, new_version: &str) -> Result<(), Box<dyn std::error::Error>> {
        news_add_pending(&self.tree, &self.path, new_version)
    }

    fn find_pending(&self) -> Option<&str> {
        news_find_pending(&self.tree, &self.path)
    }

    fn validate(&self) -> Result<(), Box<dyn std::error::Error>> {
        match self.find_pending() {
            Err(NoUnreleasedChanges) => Ok(()),
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }
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

/// Extract version info from news line.
///
/// # Arguments
///   line: Line to parse
///
/// # Returns
///   tuple with version, date released, line template, is_pending
fn parse_version_line(line: &[u8]) -> Result<(Option<&str>, Option<&str>, String, bool), Box<dyn std::error::Error>> {
    if line.strip().contains(b'\t') {
        let (version, date) = line.strip().split(b'\t', 1);
        let version_is_placeholder = check_version(version)?;
        let date_is_placeholder = check_date(date);
        let pending = version_is_placeholder || date_is_placeholder;

        return (
            if !version_is_placeholder {
                Some(version)
            } else {
                None
            },
            if !date_is_placeholder {
                Some(date)
            } else {
                None
            },
            "%(version)s\t%(date)s".to_string(),
            pending,
        );
    }

    if line.strip().contains(b' ') {
        let (mut version, mut date) = line.strip().split(b' ', 1);
        let template = if date.startswith(b'(') && date.endswith(b')') {
            date = date[1..-1];
            "%(version)s (%(date)s)".to_string()
        } else {
            "%(version)s %(date)s".to_string()
        };

        assert!(!version.is_empty());

        let version_is_placeholder = check_version(version)?;
        let date_is_placeholder = check_date(date);
        let pending = version_is_placeholder || date_is_placeholder;

        return (
            if !version_is_placeholder {
                Some(version)
            } else {
                None
            },
            if !date_is_placeholder {
                Some(date)
            } else {
                None
            },
            template,
            pending,
        );
    }

    let version = line.strip();
    let pending = check_version(version)?;
    let date_is_placeholder = pending;
    return (
        if !date_is_placeholder {
            Some(version)
        } else {
            None
        },
        None,
        "%(version)s".to_string(),
        pending,
    );
}

/// Find pending version in news file.
///
/// # Arguments
/// * `tree`: Tree object
/// * `path`: Path to news file in tree
///
/// # Returns
/// * version string
fn news_find_pending(tree: &dyn Tree, path: &Path) -> Option<&str> {
    let lines = tree.get_file_lines(path);
    let i = skip_header(&lines).ok_or("No header")?;
    let (last_version, last_date, line_format, pending) = parse_version_line(lines[i]);
    if !pending {
        return None;
    }
    last_version
}

fn news_add_pending(
    tree: &dyn MutableTree,
    path: &Path,
    new_version: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    assert!(!new_version.is_empty());

    let lines = tree.get_file_lines(path)?;
    let i = skip_header(lines).ok_or("No header")?;

    let (last_version, last_date, line_format, pending) = parse_version_line(lines[i]);
    if pending {
        return PendingExists(last_version, last_date);
    }
    lines.insert(i, b"\n".to_vec());

    let mut new_version_line = expand_template(line_format, new_version, "UNRELEASED").as_bytes().to_vec();
    new_version_line.push(b'\n');

    lines.insert(i, new_version_line);
    tree.put_file_bytes_non_atomic(path, lines.concat())?;
    Ok(())
}

fn news_mark_released(
        tree: &dyn MutableTree, path: &Path, expected_version: &str, release_date: &DateTime<Utc>) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let lines = tree.get_file_lines(path)?;
    let i = skip_header(lines).ok_or("No header")?;
    let (version, date, line_format, pending) = parse_version_line(lines[i]);
    if !pending {
        return Err(NoUnreleasedChanges());
    }
    if let Some(version) = version {
        assert!(expected_version == version, "unexpected version: {} != {}", expected_version, version);
    }
    let change_lines = Vec::new();
    for line in lines[i + 1:] {
        if (line.strip().is_empty() || line.startswith(b' ') || line.startswith(b'\t') {
            change_lines.append(line.decode())
        } else {
            break;
        }
    }
    lines[i] = expand_template(line_format, expected_version, release_date.strftime("%Y-%m-%d")).encode() + b'\n';
    tree.put_file_bytes_non_atomic(path, lines.concat())?;
    change_lines.concat()
}

fn expand_template(template: &str, version: &str, date: &str) -> String {
    template
        .replace("%(version)s", version)
        .replace("%(date)s", date)
}
