use crate::{Status, Version};
use maplit::hashmap;
use std::collections::HashMap;

fn status_tupled_version(v: &Version, s: Status) -> Option<String> {
    Some(format!(
        "({}, {}, {}, {}, 0)",
        v.major(),
        v.minor().unwrap(),
        v.micro().unwrap(),
        match s {
            Status::Final => "\"final\"",
            Status::Dev => "\"dev\"",
        }
    ))
}

fn tupled_version(v: &Version, _s: Status) -> Option<String> {
    Some(format!(
        "({}, {}, {})",
        v.major(),
        v.minor().unwrap(),
        v.micro().unwrap(),
    ))
}

fn version_major(v: &Version, _s: Status) -> Option<String> {
    Some(v.major().to_string())
}

fn version_minor(v: &Version, _s: Status) -> Option<String> {
    v.minor().map(|m| m.to_string())
}

fn version_micro(v: &Version, _s: Status) -> Option<String> {
    v.micro().map(|m| m.to_string())
}

fn version_version(v: &Version, _s: Status) -> Option<String> {
    Some(v.to_string())
}

type VersionFormatter = Box<dyn Fn(&Version, Status) -> Option<String> + Sync>;

lazy_static::lazy_static! {
    pub static ref VERSION_VARIABLES: HashMap<&'static str, VersionFormatter> = hashmap! {
        "TUPLED_VERSION" => Box::new(tupled_version) as VersionFormatter,
        "STATUS_TUPLED_VERSION" => Box::new(status_tupled_version) as VersionFormatter,
        "VERSION" => Box::new(version_version) as VersionFormatter,
        "MAJOR_VERSION" => Box::new(version_major) as VersionFormatter,
        "MINOR_VERSION" => Box::new(version_minor) as VersionFormatter,
        "MICRO_VERSION" => Box::new(version_micro) as VersionFormatter,
    };
}

pub fn expand_version_vars(
    text: &str,
    new_version: &Version,
    status: Status,
) -> Result<String, String> {
    let mut text = text.to_owned();
    for (k, vfn) in VERSION_VARIABLES.iter() {
        let var = format!("${}", k);
        if let Some(v) = vfn(new_version, status) {
            text = text.replace(var.as_str(), v.as_str());
        } else if text.contains(&var) {
            return Err(format!("no expansion for variable ${} used in {}", k, text));
        }
    }
    Ok(text)
}

#[cfg(test)]
mod expand_version_vars_tests {
    use std::str::FromStr;
    use super::expand_version_vars;
    use crate::{Status, Version};

    #[test]
    fn test_simple() {
        let text = "version = $VERSION";
        let new_version = Version::from_str("1.2.3").unwrap();
        let status = Status::Final;
        let expanded = expand_version_vars(text, &new_version, status).unwrap();
        assert_eq!(expanded, "version = 1.2.3");
    }

    #[test]
    fn test_status() {
        let text = "version = $STATUS_TUPLED_VERSION";
        let new_version = Version::from_str("1.2.3").unwrap();
        let status = Status::Dev;
        let expanded = expand_version_vars(text, &new_version, status).unwrap();
        assert_eq!(expanded, "version = (1, 2, 3, \"dev\", 0)");
    }
}

pub fn version_line_re(new_line: &str) -> regex::Regex {
    regex::Regex::new(
        lazy_regex::regex_replace_all!(
            r"\\\$([A-Z_]+)",
            regex::escape(new_line).as_str(),
            |_, var: &str| {
                if VERSION_VARIABLES.contains_key(var) {
                    format!("(?P<{}>.*)", var.to_lowercase())
                } else {
                    format!("\\${}", var)
                }
            }
        )
        .as_ref(),
    )
    .unwrap()
}

#[cfg(test)]
mod version_line_re_tests {
    use std::str::FromStr;

    #[test]
    fn test_simple() {
        let re = super::version_line_re("version = $VERSION");
        let cm = re.captures_iter("version = 1.2.3");
        let (v, s) = super::version_from_capture_matches(cm);
        assert_eq!(v, Some(super::Version::from_str("1.2.3").unwrap()));
        assert_eq!(s, None);
    }

    #[test]
    fn test_status() {
        let re = super::version_line_re("version = $STATUS_TUPLED_VERSION");
        let cm = re.captures_iter("version = (1, 2, 3, \"dev\", 0)");
        let (v, s) = super::version_from_capture_matches(cm);
        assert_eq!(v, Some(super::Version::from_str("1.2.3").unwrap()));
        assert_eq!(s, Some(super::Status::Dev));
    }
}

fn version_from_capture_matches(cm: regex::CaptureMatches) -> (Option<Version>, Option<Status>) {
    let mut major = None;
    let mut minor = None;
    let mut micro = None;
    let mut status = None;

    for c in cm {
        if let Some(v) = c.name("major_version") {
            major = Some(v.as_str().parse::<i32>().unwrap());
        }
        if let Some(v) = c.name("minor_version") {
            minor = Some(v.as_str().parse::<i32>().unwrap());
        }
        if let Some(v) = c.name("micro_version") {
            micro = Some(v.as_str().parse::<i32>().unwrap());
        }
        if let Some(v) = c.name("version") {
            let version = v.as_str().parse::<Version>().unwrap();
            major = Some(version.major());
            minor = version.minor();
            micro = version.micro();
        }
        if let Some(v) = c
            .name("tupled_version")
            .or_else(|| c.name("status_tupled_version"))
        {
            let (version, new_status) = Version::from_tupled(v.as_str()).unwrap();

            major = Some(version.major());
            minor = version.minor();
            micro = version.micro();
            if let Some(new_status) = new_status {
                status = Some(new_status);
            }
        }
    }

    if let Some(major) = major {
        (
            Some(Version {
                major,
                minor,
                micro,
            }),
            status,
        )
    } else {
        (None, None)
    }
}

/// Extracts the version and status from a line of text.
pub fn extract_version(line: &str) -> (Option<Version>, Option<Status>) {
    let re = version_line_re(line);

    version_from_capture_matches(re.captures_iter(line))
}

pub fn reverse_version(new_line: &str, lines: &[&str]) -> (Option<Version>, Option<Status>) {
    let re = version_line_re(new_line);
    for line in lines {
        let cm = re.captures_iter(line);
        let (v, s) = version_from_capture_matches(cm);
        if v.is_some() {
            return (v, s);
        }
    }
    (None, None)
}

#[cfg(test)]
mod reverse_version_tests {
    use std::str::FromStr;

    #[test]
    fn test_simple() {
        let (v, s) = super::reverse_version(
            "version = $VERSION",
            &["version = 1.2.3", "version = 1.2.4"],
        );
        assert_eq!(v, Some(super::Version::from_str("1.2.3").unwrap()));
        assert_eq!(s, None);
    }

    #[test]
    fn test_status() {
        let (v, s) = super::reverse_version(
            "version = $STATUS_TUPLED_VERSION",
            &[
                "version = (1, 2, 3, \"dev\", 0)",
                "version = (1, 2, 3, \"final\", 0)",
            ],
        );
        assert_eq!(v, Some(super::Version::from_str("1.2.3").unwrap()));
        assert_eq!(s, Some(super::Status::Dev));
    }
}

pub fn update_version_in_file(
    tree: &dyn breezyshim::tree::MutableTree,
    path: &std::path::Path,
    new_line: &str,
    r#match: Option<&str>,
    new_version: &Version,
    status: Status,
) -> Result<(), String> {
    let mut lines = tree.get_file_lines(path).unwrap();
    let mut matches = 0;
    let r = if let Some(m) = r#match {
        regex::Regex::new(m).unwrap()
    } else {
        version_line_re(new_line)
    };
    log::debug!("Expanding {:?} in {:?}", r, path);
    for oline in lines.iter_mut() {
        let line = match std::str::from_utf8(oline) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !r.is_match(line) {
            continue;
        }
        *oline = expand_version_vars(new_line, new_version, status).unwrap().into_bytes();
        matches += 1;
    }
    if matches == 0 {
        return Err(format!(
            "No matches for {} in {}",
            r.as_str(),
            path.display()
        ));
    }
    tree.put_file_bytes_non_atomic(path, lines.concat().as_slice())
        .unwrap();
    Ok(())
}

#[cfg(test)]
mod tests {
    use breezyshim::tree::Tree;
    #[test]
    fn test_update_version_in_file() {
        breezyshim::init().unwrap();
        let td = tempfile::tempdir().unwrap();
        let tree = breezyshim::controldir::ControlDir::create_standalone_workingtree(td.path(), None).unwrap();
        let path = tree.abspath(std::path::Path::new("test")).unwrap();
        std::fs::write(path.as_path(), b"version = [1.2.3]\n").unwrap();
        tree.add(&[std::path::Path::new("test")]).unwrap();
        super::update_version_in_file(
            &tree,
            path.as_path(),
            "version = [$VERSION]\n",
            None,
            &super::Version { major: 1, minor: Some(2), micro: Some(4) },
            super::Status::Final,
        )
        .unwrap();
        assert_eq!(tree.get_file_text(path.as_path()).unwrap(), b"version = [1.2.4]\n");
    }
}
