#[cfg(feature = "pyo3")]
use pyo3::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub(crate) String);

#[cfg(feature = "pyo3")]
impl ToPyObject for Version {
    fn to_object(&self, py: pyo3::Python) -> pyo3::PyObject {
        self.0.to_object(py)
    }
}

#[cfg(feature = "pyo3")]
impl IntoPy<pyo3::PyObject> for Version {
    fn into_py(self, py: pyo3::Python) -> pyo3::PyObject {
        self.0.into_py(py)
    }
}

#[cfg(feature = "pyo3")]
impl FromPyObject<'_> for Version {
    fn extract(ob: &pyo3::PyAny) -> pyo3::PyResult<Self> {
        let s = ob.extract::<String>()?;
        Ok(Version(s))
    }
}

pub fn expand_tag(tag_template: &str, version: Version) -> String {
    tag_template.replace("$VERSION", version.0.as_str())
}

pub fn unexpand_tag(tag_template: &str, tag: &str) -> Result<Version, String> {
    let tag_re = regex::Regex::new(tag_template.replace("$VERSION", "(.*)").as_str()).unwrap();
    if let Some(m) = tag_re.captures(tag) {
        Ok(Version(m.get(1).unwrap().as_str().to_string()))
    } else {
        Err(format!(
            "Tag {} does not match template {}",
            tag, tag_template
        ))
    }
}

pub fn increase_version(version: &mut Version, idx: isize) {
    // Split the version string by '.' and collect each part into a Vec<i32>
    // The `unwrap_or(0)` is there to handle the case where the string is not a valid integer.
    let mut parts: Vec<i32> = version
        .0
        .split('.')
        .map(|x| x.parse::<i32>().unwrap_or(0))
        .collect();

    // Calculate the index to modify.
    // We use `wrapping_add` to gracefully handle negative indices by making them wrap around to the end.
    let idx = ((idx.wrapping_add(parts.len() as isize)) as usize) % parts.len();

    parts[idx] += 1;

    // Convert each element back to a String and join them with '.'.
    // Return the resulting String.
    version.0 = parts
        .into_iter()
        .map(|x| x.to_string())
        .collect::<Vec<String>>()
        .join(".");
}
