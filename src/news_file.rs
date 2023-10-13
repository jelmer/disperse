use lazy_regex::regex_is_match;

#[derive(Debug)]
pub struct OddVersion(String);

impl std::fmt::Display for OddVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "Odd version: {}", self.0)
    }
}

impl std::error::Error for OddVersion {}

pub fn check_date(d: &str) -> bool {
    d == "UNRELEASED" || d.starts_with("NEXT ")
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

pub fn expand_template(template: &str, version: &str, date: &str) -> String {
    template
        .replace("%(version)s", version)
        .replace("%(date)s", date)
}

pub fn skip_header<'a>(lines: &mut impl Iterator<Item = &'a [u8]>) {
    let mut iter = lines.peekable();
    while let Some(line) = iter.peek() {
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
        break;
    }
}

/// Find pending version in news file.
///
/// # Arguments
/// * `tree`: Tree object
/// * `path`: Path to news file in tree
///
/// # Returns
/// * version string
pub fn news_find_pending(
    tree: &dyn breezyshim::tree::Tree,
    path: &std::path::Path,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let lines = tree.get_file_lines(path).unwrap();
    let mut iter = lines.iter().map(|x| x.as_slice());
    skip_header(&mut iter);
    let line = String::from_utf8(iter.next().unwrap().to_vec())?;
    let (last_version, _last_date, _line_format, pending) = parse_version_line(line.as_str())?;
    if !pending {
        return Ok(None);
    }
    Ok(last_version.map(|v| v.to_string()))
}

/// Extract version info from news line.
///
/// # Arguments
///   line: Line to parse
///
/// # Returns
///   tuple with version, date released, line template, is_pending
fn parse_version_line(
    line: &str,
) -> Result<(Option<&str>, Option<&str>, String, bool), Box<dyn std::error::Error>> {
    // Strip leading and trailing whitespace
    let line = line.trim();

    if line.contains('\t') {
        if let Some((version, date)) = line.split_once('\t') {
            let version_is_placeholder = check_version(version)?;
            let date_is_placeholder = check_date(date);
            let pending = version_is_placeholder || date_is_placeholder;

            return Ok((
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
            ));
        }
    }

    if line.contains(' ') {
        if let Some((version, mut date)) = line.split_once(' ') {
            let template = if date.starts_with('(') && date.ends_with(')') {
                date = &date[1..date.len() - 1];
                "%(version)s (%(date)s)".to_string()
            } else {
                "%(version)s %(date)s".to_string()
            };

            assert!(!version.is_empty());

            let version_is_placeholder = check_version(version)?;
            let date_is_placeholder = check_date(date);
            let pending = version_is_placeholder || date_is_placeholder;

            return Ok((
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
            ));
        }
    }

    let version = line;
    let pending = check_version(version)?;
    let date_is_placeholder = pending;
    Ok((
        if !date_is_placeholder {
            Some(version)
        } else {
            None
        },
        None,
        "%(version)s".to_string(),
        pending,
    ))
}
