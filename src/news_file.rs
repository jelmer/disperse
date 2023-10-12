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
