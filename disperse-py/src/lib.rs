use pyo3::prelude::*;

#[pyfunction]
fn cargo_publish(tree: PyObject, subpath: std::path::PathBuf) -> PyResult<()> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    disperse::cargo::publish(&tree, subpath.as_path()).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("cargo publish failed: {}", e))
    })
}

#[pyfunction]
fn find_version_in_cargo(tree: PyObject) -> PyResult<String> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    disperse::cargo::find_version(&tree).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
            "find_version_in_cargo failed: {}",
            e
        ))
    })
}

#[pyfunction]
fn update_version_in_cargo(tree: PyObject, version: String) -> PyResult<()> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    disperse::cargo::update_version(&tree, &version).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!(
            "update_version_in_cargo failed: {}",
            e
        ))
    })
}

#[pymodule]
fn _disperse_rs(_py: Python, m: &PyModule) -> PyResult<()> {
    m.add_wrapped(wrap_pyfunction!(cargo_publish))?;
    m.add_wrapped(wrap_pyfunction!(find_version_in_cargo))?;
    m.add_wrapped(wrap_pyfunction!(update_version_in_cargo))?;
    Ok(())
}
