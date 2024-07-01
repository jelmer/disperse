use crate::Version;
use breezyshim::tree::MutableTree;
use lazy_regex::regex_is_match;

pub fn check_date(d: &str) -> bool {
    d == "UNRELEASED" || d.starts_with("NEXT ")
}

pub fn check_version(v: &str) -> Result<bool, Error> {
    if v == "UNRELEASED" || v == "%(version)s" || v == "NEXT" {
        return Ok(true);
    }

    if !regex_is_match!(r"^[0-9\.]+$", v) {
        return Err(Error::OddVersion(v.to_string()));
    }

    Ok(false)
}

pub fn expand_template(template: &str, version: &Version, date: &str) -> String {
    template
        .replace("%(version)s", version.to_string().as_str())
        .replace("%(date)s", date)
}

pub fn skip_header<'a, I: Iterator<Item = &'a [u8]>>(iter: &mut std::iter::Peekable<I>) -> usize {
    let mut i = 0;
    while let Some(line) = iter.peek() {
        i += 1;
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
    i as usize
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
) -> Result<Option<String>, Error> {
    let lines = tree.get_file_lines(path)?;
    let mut iter = lines.iter().map(|x| x.as_slice()).peekable();
    skip_header(&mut iter);
    let line = String::from_utf8(iter.next().unwrap().to_vec())
        .map_err(|_| Error::InvalidData("Invalid UTF-8 in news file".to_string()))?;
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
fn parse_version_line(line: &str) -> Result<(Option<&str>, Option<&str>, String, bool), Error> {
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

pub fn news_add_pending(
    tree: &dyn breezyshim::tree::MutableTree,
    path: &std::path::Path,
    new_version: &crate::Version,
) -> Result<(), Error> {
    let mut lines = tree.get_file_lines(path)?;
    let mut line_iter = lines.iter().map(|x| x.as_slice()).peekable();
    let i = skip_header(&mut line_iter);

    let line = String::from_utf8(line_iter.next().unwrap().to_vec())
        .map_err(|_| Error::InvalidData("Invalid UTF-8 in news file".to_string()))?;

    let (last_version, last_date, line_format, pending) = parse_version_line(line.as_str())?;
    if pending {
        let last_date = last_date
            .map(|x| x.parse().map_err(|_| Error::InvalidData(x.to_string())))
            .transpose()?;
        return Err(Error::PendingExists {
            last_version: last_version
                .unwrap()
                .parse()
                .map_err(|_| Error::InvalidData(last_version.unwrap().to_string()))?,
            last_date,
        });
    }
    lines.insert(i, b"\n".to_vec());

    let mut new_version_line = expand_template(line_format.as_str(), new_version, "UNRELEASED")
        .as_bytes()
        .to_vec();
    new_version_line.push(b'\n');

    lines.insert(i, new_version_line);
    tree.put_file_bytes_non_atomic(path, lines.concat().as_slice())?;
    Ok(())
}

#[derive(Debug)]
pub struct NoUnreleasedChanges();

impl std::fmt::Display for NoUnreleasedChanges {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "No unreleased changes")
    }
}

impl std::error::Error for NoUnreleasedChanges {}

#[derive(Debug)]
pub enum Error {
    BrzError(breezyshim::error::Error),
    NoUnreleasedChanges,
    OddVersion(String),
    PendingExists {
        last_version: Version,
        last_date: Option<chrono::NaiveDate>,
    },
    InvalidData(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self {
            Self::BrzError(e) => write!(f, "Tree error: {}", e),
            Self::NoUnreleasedChanges => write!(f, "No unreleased changes"),
            Self::OddVersion(s) => write!(f, "Odd version: {}", s),
            Self::PendingExists {
                last_version,
                last_date,
            } => {
                write!(
                    f,
                    "Pending version already exists: {} {}",
                    last_version.to_string(),
                    last_date.map_or_else(
                        || "UNRELEASED".to_string(),
                        |x| x.format("%Y-%m-%d").to_string()
                    )
                )
            }
            Self::InvalidData(s) => write!(f, "Invalid data: {}", s),
        }
    }
}

impl std::error::Error for Error {}

impl From<breezyshim::error::Error> for Error {
    fn from(e: breezyshim::error::Error) -> Self {
        Self::BrzError(e)
    }
}

pub fn news_mark_released(
    tree: &dyn MutableTree,
    path: &std::path::Path,
    expected_version: &Version,
    release_date: &chrono::NaiveDate,
) -> Result<String, Error> {
    let mut lines = tree.get_file_lines(path)?;
    let mut iter = lines.iter().map(|x| x.as_slice()).peekable();
    let i = skip_header(&mut iter);
    let line = String::from_utf8(iter.next().unwrap().to_vec())
        .map_err(|_| Error::InvalidData("Invalid UTF-8 in news file".to_string()))?;
    let (version, _date, line_format, pending) = parse_version_line(line.as_str())?;
    if !pending {
        return Err(Error::NoUnreleasedChanges);
    }
    if let Some(version) = version {
        assert_eq!(
            expected_version.to_string().as_str(),
            version,
            "unexpected version: {} != {}",
            expected_version.to_string(),
            version
        );
    }
    let mut change_lines = Vec::new();
    for line in lines[i + 1..].iter() {
        let line = match String::from_utf8(line.to_vec()) {
            Ok(line) => line,
            Err(_) => {
                continue;
            }
        };
        if line.trim().is_empty() || line.starts_with(' ') || line.starts_with('\t') {
            change_lines.push(line);
        } else {
            break;
        }
    }
    let new_line = expand_template(
        line_format.as_str(),
        expected_version,
        release_date.format("%Y-%m-%d").to_string().as_str(),
    ) + "\n";
    lines[i] = new_line.into_bytes();

    tree.put_file_bytes_non_atomic(path, lines.concat().as_slice())?;
    Ok(change_lines.concat())
}

pub struct NewsFile<'a> {
    tree: &'a breezyshim::tree::WorkingTree,
    path: std::path::PathBuf,
}

impl<'a> NewsFile<'a> {
    pub fn new(
        tree: &'a breezyshim::tree::WorkingTree,
        path: &std::path::Path,
    ) -> Result<Self, Error> {
        Ok(Self {
            tree,
            path: path.to_path_buf(),
        })
    }

    pub fn add_pending(&self, new_version: &crate::Version) -> Result<(), Error> {
        news_add_pending(self.tree, self.path.as_path(), new_version)
    }

    pub fn mark_released(
        &self,
        expected_version: &Version,
        release_date: &chrono::NaiveDate,
    ) -> Result<String, Error> {
        news_mark_released(
            self.tree,
            self.path.as_path(),
            expected_version,
            release_date,
        )
    }
}
