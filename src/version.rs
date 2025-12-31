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
        let major = parts[0]
            .parse::<i32>()
            .map_err(|e| format!("invalid major version: {}", e))?;
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

    pub fn from_tupled(text: &str) -> Result<(Self, Option<crate::Status>), Error> {
        if text.starts_with('(') && text.ends_with(')') {
            return Self::from_tupled(&text[1..text.len() - 1]);
        }
        let parts: Vec<&str> = text.split(',').collect();
        if parts.is_empty() || parts.len() > 5 {
            return Err(Error(format!("invalid version: {}", text)));
        }
        let major = parts[0]
            .trim()
            .parse::<i32>()
            .map_err(|e| Error(format!("invalid major version: {}", e)))?;
        let minor = parts
            .get(1)
            .map(|x| x.trim().parse::<i32>())
            .transpose()
            .map_err(|e| Error(format!("invalid minor version: {}", e)))?;
        let micro = parts
            .get(2)
            .map(|x| x.trim().parse::<i32>())
            .transpose()
            .map_err(|e| Error(format!("invalid micro version: {}", e)))?;
        let status = if let Some(s) = parts.get(3).map(|x| x.trim()) {
            if s == "\"dev\"" || s == "'dev'" {
                Some(crate::Status::Dev)
            } else if s == "\"final\"" || s == "'final'" {
                Some(crate::Status::Final)
            } else {
                return Err(Error(format!("invalid status: {}", s)));
            }
        } else {
            None
        };
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_from_tupled() {
        assert_eq!(
            Version::from_tupled("(1, 2, 3, \"dev\", 0)").unwrap(),
            (
                Version {
                    major: 1,
                    minor: Some(2),
                    micro: Some(3),
                },
                Some(crate::Status::Dev)
            )
        );
        assert_eq!(
            Version::from_tupled("(1, 2, 3)").unwrap(),
            (
                Version {
                    major: 1,
                    minor: Some(2),
                    micro: Some(3),
                },
                None
            )
        );
        assert_eq!(
            Version::from_tupled("(1, 2)").unwrap(),
            (
                Version {
                    major: 1,
                    minor: Some(2),
                    micro: None,
                },
                None
            )
        );
        assert_eq!(
            Version::from_tupled("(1)").unwrap(),
            (
                Version {
                    major: 1,
                    minor: None,
                    micro: None,
                },
                None
            )
        );
        assert_eq!(
            Version::from_tupled("1").unwrap(),
            (
                Version {
                    major: 1,
                    minor: None,
                    micro: None,
                },
                None
            )
        );

        // Test error cases to catch logic errors
        assert!(Version::from_tupled("").is_err());
        assert!(Version::from_tupled("not_a_number").is_err());
        assert!(Version::from_tupled("(1, 2, not_a_number)").is_err());
    }

    #[test]
    fn test_increase_version_major() {
        let mut v = Version {
            major: 1,
            minor: Some(2),
            micro: Some(3),
        };
        increase_version(&mut v, 0);
        assert_eq!(v.major, 2);
        assert_eq!(v.minor, Some(2));
        assert_eq!(v.micro, Some(3));
    }

    #[test]
    fn test_increase_version_minor() {
        let mut v = Version {
            major: 1,
            minor: Some(2),
            micro: Some(3),
        };
        increase_version(&mut v, 1);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, Some(3));
        assert_eq!(v.micro, Some(3));

        // Test when minor is None
        let mut v2 = Version {
            major: 1,
            minor: None,
            micro: Some(3),
        };
        increase_version(&mut v2, 1);
        assert_eq!(v2.major, 1);
        assert_eq!(v2.minor, Some(1));
        assert_eq!(v2.micro, Some(3));
    }

    #[test]
    fn test_increase_version_micro() {
        let mut v = Version {
            major: 1,
            minor: Some(2),
            micro: Some(3),
        };
        increase_version(&mut v, 2);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, Some(2));
        assert_eq!(v.micro, Some(4));

        // Test when micro is None
        let mut v2 = Version {
            major: 1,
            minor: Some(2),
            micro: None,
        };
        increase_version(&mut v2, 2);
        assert_eq!(v2.major, 1);
        assert_eq!(v2.minor, Some(2));
        assert_eq!(v2.micro, Some(1));
    }

    #[test]
    fn test_increase_version_auto() {
        // Test -1 index (auto increment rightmost component)
        let mut v = Version {
            major: 1,
            minor: Some(2),
            micro: Some(3),
        };
        increase_version(&mut v, -1);
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, Some(2));
        assert_eq!(v.micro, Some(4));

        // Test when micro is None but minor exists
        let mut v2 = Version {
            major: 1,
            minor: Some(2),
            micro: None,
        };
        increase_version(&mut v2, -1);
        assert_eq!(v2.major, 1);
        assert_eq!(v2.minor, Some(3));
        assert_eq!(v2.micro, None);

        // Test when both minor and micro are None
        let mut v3 = Version {
            major: 1,
            minor: None,
            micro: None,
        };
        increase_version(&mut v3, -1);
        assert_eq!(v3.major, 2);
        assert_eq!(v3.minor, None);
        assert_eq!(v3.micro, None);
    }

    #[test]
    fn test_expand_tag() {
        let v = Version {
            major: 1,
            minor: Some(2),
            micro: Some(3),
        };
        assert_eq!(expand_tag("v$VERSION", &v), "v1.2.3");
        assert_eq!(expand_tag("release-$VERSION", &v), "release-1.2.3");
        assert_eq!(expand_tag("$VERSION", &v), "1.2.3");
    }

    #[test]
    fn test_unexpand_tag() {
        let result = unexpand_tag("v$VERSION", "v1.2.3").unwrap();
        assert_eq!(result.major, 1);
        assert_eq!(result.minor, Some(2));
        assert_eq!(result.micro, Some(3));

        let result2 = unexpand_tag("release-$VERSION", "release-2.0.0").unwrap();
        assert_eq!(result2.major, 2);
        assert_eq!(result2.minor, Some(0));
        assert_eq!(result2.micro, Some(0));

        // Test error case
        assert!(unexpand_tag("v$VERSION", "1.2.3").is_err());
        assert!(unexpand_tag("v$VERSION", "v-invalid").is_err());
    }

    #[test]
    fn test_version_display() {
        let v1 = Version {
            major: 1,
            minor: Some(2),
            micro: Some(3),
        };
        assert_eq!(v1.to_string(), "1.2.3");

        let v2 = Version {
            major: 1,
            minor: Some(2),
            micro: None,
        };
        assert_eq!(v2.to_string(), "1.2");

        let v3 = Version {
            major: 1,
            minor: None,
            micro: None,
        };
        assert_eq!(v3.to_string(), "1");
    }

    #[test]
    fn test_version_major() {
        let v1 = Version {
            major: 5,
            minor: Some(2),
            micro: Some(3),
        };
        assert_eq!(v1.major(), 5);

        let v2 = Version {
            major: 0,
            minor: None,
            micro: None,
        };
        assert_eq!(v2.major(), 0);
    }

    #[test]
    fn test_error_display() {
        let err = Error("test error message".to_string());
        assert_eq!(err.to_string(), "test error message");
        assert_eq!(format!("{}", err), "test error message");
    }
}

#[cfg(feature = "pyo3")]
impl<'py> pyo3::IntoPyObject<'py> for Version {
    type Target = pyo3::types::PyString;
    type Output = pyo3::Bound<'py, Self::Target>;
    type Error = std::convert::Infallible;

    fn into_pyobject(self, py: pyo3::Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pyo3::types::PyString::new(py, &self.to_string()))
    }
}

#[cfg(feature = "pyo3")]
impl<'py> pyo3::IntoPyObject<'py> for &Version {
    type Target = pyo3::types::PyString;
    type Output = pyo3::Bound<'py, Self::Target>;
    type Error = std::convert::Infallible;

    fn into_pyobject(self, py: pyo3::Python<'py>) -> Result<Self::Output, Self::Error> {
        Ok(pyo3::types::PyString::new(py, &self.to_string()))
    }
}

#[cfg(feature = "pyo3")]
impl<'py> FromPyObject<'_, 'py> for Version {
    type Error = pyo3::PyErr;

    fn extract(ob: pyo3::Borrowed<'_, 'py, pyo3::PyAny>) -> Result<Self, Self::Error> {
        let s = ob.extract::<String>()?;
        Version::from_str(s.as_str())
            .map_err(|e| pyo3::exceptions::PyValueError::new_err(format!("Invalid version: {}", e)))
    }
}

pub fn expand_tag(tag_template: &str, version: &Version) -> String {
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

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.fmt(f)?;
        Ok(())
    }
}

impl From<std::num::ParseIntError> for Error {
    fn from(e: std::num::ParseIntError) -> Self {
        Error(format!("{}", e))
    }
}

impl std::error::Error for Error {}
