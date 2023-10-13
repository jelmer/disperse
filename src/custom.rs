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
    for oline in lines.iter_mut() {
        let line = match String::from_utf8(oline.to_vec()) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if !r.is_match(line.as_str()) {
            continue;
        }
        let new_line = expand_version_vars(line.as_str(), new_version, status).unwrap();
        *oline = vec![new_line, "\n".to_string()].concat().into_bytes();
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
