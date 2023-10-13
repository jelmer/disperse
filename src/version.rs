use std::str::FromStr;

#[cfg(feature = "pyo3")]
use pyo3::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: i32,
    pub minor: Option<i32>,
    pub micro: Option<i32>,
}

impl std::str::FromStr for Version {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parts: Vec<&str> = s.split('.').collect();
        let major = parts[0].parse::<i32>().unwrap();
        let minor = parts.get(1).map(|x| x.parse::<i32>().unwrap());
        let micro = parts.get(2).map(|x| x.parse::<i32>().unwrap());
        Ok(Version {
            major,
            minor,
            micro,
        })
    }
}

impl ToString for Version {
    fn to_string(&self) -> String {
        let mut s = self.major.to_string();
        if let Some(minor) = self.minor {
            s.push_str(format!(".{}", minor).as_str());
        }
        if let Some(micro) = self.micro {
            s.push_str(format!(".{}", micro).as_str());
        }
        s
    }
}

impl Version {
    pub fn major(&self) -> i32 {
        self.major
    }

    pub fn minor(&self) -> Option<i32> {
        self.minor
    }

    pub fn micro(&self) -> Option<i32> {
        self.micro
    }

    pub fn from_tupled(text: &str) -> Result<(Self, Option<crate::Status>), String> {
        if text.starts_with('(') && text.ends_with(')') {
            return Self::from_tupled(&text[1..text.len() - 1]);
        }
        let parts: Vec<&str> = text.split(',').collect();
        let major = parts[0].parse::<i32>().unwrap();
        let minor = parts.get(1).and_then(|x| x.parse::<i32>().ok());
        let micro = parts.get(2).and_then(|x| x.parse::<i32>().ok());
        let status = parts.get(3).and_then(|x| x.parse::<crate::Status>().ok());
        Ok((
            Version {
                major,
                minor,
                micro,
            },
            status,
        ))
    }
}

#[cfg(feature = "pyo3")]
impl ToPyObject for Version {
    fn to_object(&self, py: pyo3::Python) -> pyo3::PyObject {
        self.to_string().to_object(py)
    }
}

#[cfg(feature = "pyo3")]
impl IntoPy<pyo3::PyObject> for Version {
    fn into_py(self, py: pyo3::Python) -> pyo3::PyObject {
        self.to_string().into_py(py)
    }
}

#[cfg(feature = "pyo3")]
impl FromPyObject<'_> for Version {
    fn extract(ob: &pyo3::PyAny) -> pyo3::PyResult<Self> {
        let s = ob.extract::<String>()?;
        Version::from_str(s.as_str())
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("Invalid version: {}", e)))
    }
}

pub fn expand_tag(tag_template: &str, version: Version) -> String {
    tag_template.replace("$VERSION", version.to_string().as_str())
}

pub fn unexpand_tag(tag_template: &str, tag: &str) -> Result<Version, String> {
    let tag_re = regex::Regex::new(tag_template.replace("$VERSION", "(.*)").as_str()).unwrap();
    if let Some(m) = tag_re.captures(tag) {
        Ok(Version::from_str(m.get(1).unwrap().as_str()).map_err(|e| {
            format!(
                "Tag {} does not match template {}: {}",
                tag, tag_template, e
            )
        })?)
    } else {
        Err(format!(
            "Tag {} does not match template {}",
            tag, tag_template
        ))
    }
}

pub fn increase_version(version: &mut Version, idx: isize) {
    match idx {
        0 => version.major += 1,
        1 => {
            if let Some(minor) = version.minor.as_mut() {
                *minor += 1;
            } else {
                version.minor = Some(1);
            }
        }
        2 => {
            if let Some(micro) = version.micro.as_mut() {
                *micro += 1;
            } else {
                version.micro = Some(1);
            }
        }
        -1 => {
            if let Some(micro) = version.micro.as_mut() {
                *micro += 1;
            } else if let Some(minor) = version.minor.as_mut() {
                *minor += 1;
            } else {
                version.major += 1;
            }
        }
        _ => panic!("Invalid index {}", idx),
    }
}
