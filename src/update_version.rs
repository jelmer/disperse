use crate::Version;
use breezyshim::tree::MutableTree;
use chrono::{DateTime, Utc};
use regex::Regex;
use std::error::Error;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

pub fn update_version_in_manpage(
    tree: &mut dyn MutableTree,
    path: &Path,
    new_version: &Version,
    release_date: DateTime<Utc>,
) -> Result<(), Box<dyn Error>> {
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
            if let Some(captures) = re.captures(&args[3]) {
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
                let formatted_version =
                    re.replace(version_str, f.replace("$VERSION", new_version.0.as_str()));
                args[4] = formatted_version.to_string();
                break;
            }
        }

        let updated_line = shlex::join(args.iter().map(|s| s.as_ref()));
        lines[i] = updated_line.into_bytes();
        break;
    }

    if lines.iter().all(|line| !line.starts_with(&b".TH "[..])) {
        return Err(format!("No matches for date or version in {}", path.display()).into());
    }

    tree.put_file_bytes_non_atomic(path, &lines.concat())?;

    Ok(())
}
