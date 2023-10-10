pub mod cargo;

pub struct Version(String);

#[cfg(feature = "pyo3")]
impl pyo3::ToPyObject for Version {
    fn to_object(&self, py: pyo3::Python) -> pyo3::PyObject {
        self.0.to_object(py)
    }
}

#[cfg(feature = "pyo3")]
impl pyo3::FromPyObject<'_> for Version {
    fn extract(ob: &pyo3::PyAny) -> pyo3::PyResult<Self> {
        let s = ob.extract::<String>()?;
        Ok(Version(s))
    }
}

pub fn expand_tag(tag_template: &str, version: Version) -> String {
    tag_template.replace("$VERSION", version.0.as_str())
}
