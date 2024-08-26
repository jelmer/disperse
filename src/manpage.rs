use crate::Version;
use breezyshim::tree::MutableTree;
use chrono::NaiveDate;
use regex::Regex;
use std::str::FromStr;

use std::io::{BufRead, BufReader};
use std::path::Path;

#[derive(Debug)]
pub enum Error {
    BrzError(breezyshim::error::Error),
    IoError(std::io::Error),
    InvalidRegex(regex::Error),
    NoMatches,
}

impl From<breezyshim::error::Error> for Error {
    fn from(e: breezyshim::error::Error) -> Self {
        Error::BrzError(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl From<regex::Error> for Error {
    fn from(e: regex::Error) -> Self {
        Error::InvalidRegex(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self {
            Error::BrzError(e) => write!(f, "TreeError: {}", e),
            Error::IoError(e) => write!(f, "IoError: {}", e),
            Error::InvalidRegex(e) => write!(f, "InvalidRegex: {}", e),
            Error::NoMatches => write!(f, "NoMatches"),
        }
    }
}

impl std::error::Error for Error {}

/// Update the version in a manpage.
pub fn update_version_in_manpage(
    tree: &dyn MutableTree,
    path: &Path,
    new_version: &Version,
    release_date: NaiveDate,
) -> Result<(), Error> {
    let file = tree.get_file(path)?;

    let mut lines = BufReader::new(file)
        .split(b'\n')
        .collect::<Result<Vec<_>, _>>()?;

    let date_options: Vec<(&str, &str)> = vec![
        (r"20[0-9][0-9]-[0-1][0-9]-[0-3][0-9]", "%Y-%m-%d"),
        (r"[A-Za-z]+ ([0-9]{4})", "%B %Y"),
    ];

    let version_options: Vec<(&str, &str)> = vec![(r"([^ ]+) ([0-9a-z.]+)", r"\1 $VERSION")];

    for (i, line) in lines.iter_mut().enumerate() {
        if !line.starts_with(&b".TH "[..]) {
            continue;
        }

        let mut args = match shlex::split(String::from_utf8_lossy(line).as_ref()) {
            Some(args) => args,
            None => continue,
        };

        // Iterate through date options
        for (r, f) in &date_options {
            let re = Regex::new(r)?;
            if let Some(_captures) = re.captures(&args[3]) {
                let formatted_date = release_date.format(f).to_string();
                args[3] = formatted_date;
                break;
            }
        }

        // Iterate through version options
        for (r, f) in &version_options {
            let re = Regex::new(r)?;
            if let Some(captures) = re.captures(&args[4]) {
                let version_str = captures.get(0).unwrap().as_str();
                let formatted_version = re.replace(
                    version_str,
                    f.replace("$VERSION", new_version.to_string().as_str()),
                );
                args[4] = formatted_version.to_string();
                break;
            }
        }

        let updated_line = shlex::try_join(args.iter().map(|s| s.as_ref())).unwrap();
        lines[i] = updated_line.into_bytes();
        break;
    }

    if lines.iter().all(|line| !line.starts_with(&b".TH "[..])) {
        return Err(Error::NoMatches);
    }

    tree.put_file_bytes_non_atomic(path, &lines.concat())?;

    Ok(())
}

/// Validate that a manpage is updateable.
fn validate_manpage_updateable(bufread: &mut dyn BufRead) -> Result<(), Error> {
    let mut lines = bufread.split(b'\n').collect::<Result<Vec<_>, _>>()?;

    let mut found = false;
    for line in lines.iter_mut() {
        if !line.starts_with(&b".TH "[..]) {
            continue;
        }

        let args = match shlex::split(String::from_utf8_lossy(line).as_ref()) {
            Some(args) => args,
            None => continue,
        };

        if args.len() < 5 {
            continue;
        }

        if let Some((_, version)) = args[4].split_once(' ') {
            if Version::from_str(version).is_ok() {
                found = true;
                break;
            }
        }
    }

    if !found {
        return Err(Error::NoMatches);
    }

    Ok(())
}

pub fn validate_update_manpage(
    tree: &dyn breezyshim::tree::Tree,
    update_manpage: &Path,
) -> Result<(), Error> {
    let file = tree.get_file(update_manpage)?;

    validate_manpage_updateable(&mut BufReader::new(file))
}

#[cfg(test)]
mod tests {
    #[test]
    fn test_validate_manpage_updateable() {
        let b = b".TH BZR 1 \"2019-12-31\" \"Bazaar 2.7.0\" \"Bazaar Reference Manual\"\n";
        super::validate_manpage_updateable(&mut std::io::Cursor::new(b)).unwrap();
    }
}
