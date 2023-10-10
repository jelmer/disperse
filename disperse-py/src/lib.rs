use breezyshim::branch::Branch;
use breezyshim::tree::{Tree, WorkingTree};
use disperse::Version;
use pyo3::prelude::*;
use std::path::Path;

#[pyfunction]
fn check_new_revisions(branch: PyObject, news_file: Option<std::path::PathBuf>) -> PyResult<bool> {
    let branch = breezyshim::branch::RegularBranch::new(branch);
    disperse::check_new_revisions(&branch, news_file.as_ref().map(Path::new))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn increase_version(mut version: Version, part: Option<isize>) -> PyResult<Version> {
    let part = part.unwrap_or(-1);
    disperse::increase_version(&mut version, part);
    Ok(version)
}

#[pyfunction]
fn expand_tag(template: &str, version: Version) -> PyResult<String> {
    Ok(disperse::expand_tag(template, version))
}

#[pyfunction]
fn get_owned_crates(user: &str) -> PyResult<Vec<String>> {
    disperse::cargo::get_owned_crates(user).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("get_owned_crates failed: {}", e))
    })
}

#[pyfunction]
fn cargo_publish(tree: PyObject, subpath: std::path::PathBuf) -> PyResult<()> {
    let tree = WorkingTree::new(tree)?;
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
    m.add_wrapped(wrap_pyfunction!(get_owned_crates))?;
    m.add_wrapped(wrap_pyfunction!(expand_tag))?;
    m.add_wrapped(wrap_pyfunction!(increase_version))?;
    m.add_wrapped(wrap_pyfunction!(check_new_revisions))?;
    Ok(())
}
