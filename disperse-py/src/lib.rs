use breezyshim::branch::Branch;
use breezyshim::tree::{MutableTree, Tree, WorkingTree};
use disperse::Version;
use pyo3::create_exception;
use pyo3::prelude::*;
use std::path::Path;

#[pyfunction]
fn pypi_discover_urls(pypi_user: &str) -> PyResult<Vec<String>> {
    disperse::python::pypi_discover_urls(pypi_user).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("discover_urls failed: {}", e))
    })
}

#[pyfunction]
fn update_version_in_manpage(
    tree: PyObject,
    path: std::path::PathBuf,
    new_version: Version,
    release_date: chrono::DateTime<chrono::Utc>,
) -> PyResult<()> {
    let mut tree = WorkingTree::new(tree).unwrap();

    disperse::manpage::update_version_in_manpage(
        &mut tree,
        path.as_path(),
        &new_version,
        release_date,
    )
    .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn update_version_in_pyproject_toml(tree: PyObject, new_version: Version) -> PyResult<bool> {
    let tree = WorkingTree::new(tree).unwrap();

    disperse::python::update_version_in_pyproject_toml(&tree, &new_version)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn check_new_revisions(branch: PyObject, news_file: Option<std::path::PathBuf>) -> PyResult<bool> {
    let branch = breezyshim::branch::RegularBranch::new(branch);
    disperse::check_new_revisions(&branch, news_file.as_ref().map(Path::new))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn increase_version(mut version: Version, part: Option<isize>) -> PyResult<Version> {
    let part = part.unwrap_or(-1);
    disperse::version::increase_version(&mut version, part);
    Ok(version)
}

#[pyfunction]
fn expand_tag(template: &str, version: Version) -> PyResult<String> {
    Ok(disperse::version::expand_tag(template, version))
}

#[pyfunction]
fn unexpand_tag(template: &str, tag: &str) -> PyResult<Version> {
    disperse::version::unexpand_tag(template, tag).map_err(|e| {
        PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("unexpand_tag failed: {}", e))
    })
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
fn find_version_in_pyproject_toml(tree: PyObject) -> PyResult<Option<Version>> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    Ok(disperse::python::find_version_in_pyproject_toml(&tree))
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

#[pyfunction]
fn find_last_version_in_tags(
    branch: PyObject,
    tag_name: &str,
) -> (Option<Version>, Option<disperse::Status>) {
    let branch = breezyshim::branch::RegularBranch::new(branch);
    let (version, status) = disperse::find_last_version_in_tags(&branch, tag_name);
    (version, status)
}

#[pyfunction]
fn find_name_in_pyproject_toml(tree: PyObject) -> PyResult<Option<String>> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    Ok(disperse::python::find_name_in_pyproject_toml(&tree))
}

#[pyfunction]
fn pyproject_uses_hatch_vcs(tree: PyObject) -> PyResult<bool> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    disperse::python::pyproject_uses_hatch_vcs(&tree)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn find_hatch_vcs_version(tree: PyObject) -> PyResult<Option<Version>> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;
    Ok(disperse::python::find_hatch_vcs_version(&tree))
}

#[pyfunction]
fn read_project_urls_from_setup_cfg(
    path: std::path::PathBuf,
) -> PyResult<Vec<(String, Option<String>)>> {
    disperse::python::read_project_urls_from_setup_cfg(path.as_path())
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
        .map(|urls| {
            urls.into_iter()
                .map(|(url, branch)| (url.to_string(), branch))
                .collect()
        })
}

#[pyfunction]
fn read_project_urls_from_pyproject_toml(
    path: std::path::PathBuf,
) -> PyResult<Vec<(String, Option<String>)>> {
    disperse::python::read_project_urls_from_pyproject_toml(path.as_path())
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
        .map(|urls| {
            urls.into_iter()
                .map(|(url, branch)| (url.to_string(), branch))
                .collect()
        })
}

create_exception!(
    disperse.python,
    UploadCommandFailed,
    pyo3::exceptions::PyRuntimeError
);

#[pyfunction]
fn upload_python_artifacts(tree: PyObject, pypi_paths: Vec<String>) -> PyResult<()> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;

    disperse::python::upload_python_artifacts(
        &tree,
        pypi_paths
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .as_slice(),
    )
    .map_err(|e| UploadCommandFailed::new_err(format!("upload_python_artifacts failed: {}", e)))
}

#[pyfunction]
fn create_setup_py_artifacts(tree: PyObject) -> PyResult<Vec<String>> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;

    disperse::python::create_setup_py_artifacts(&tree)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn create_python_artifacts(tree: PyObject) -> PyResult<Vec<std::path::PathBuf>> {
    let tree = breezyshim::tree::WorkingTree::new(tree)?;

    disperse::python::create_python_artifacts(&tree)
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e.to_string()))
}

#[pyfunction]
fn check_date(date: &str) -> PyResult<bool> {
    Ok(disperse::news_file::check_date(date))
}

pyo3::import_exception!(disperse.news_file, OddVersion);

#[pyfunction]
fn check_version(version: &str) -> PyResult<bool> {
    disperse::news_file::check_version(version)
        .map_err(|e| OddVersion::new_err(format!("check_version failed: {}", e)))
}

#[pymodule]
fn _disperse_rs(py: Python, m: &PyModule) -> PyResult<()> {
    m.add_wrapped(wrap_pyfunction!(cargo_publish))?;
    m.add_wrapped(wrap_pyfunction!(find_version_in_cargo))?;
    m.add_wrapped(wrap_pyfunction!(find_version_in_pyproject_toml))?;
    m.add_wrapped(wrap_pyfunction!(update_version_in_cargo))?;
    m.add_wrapped(wrap_pyfunction!(get_owned_crates))?;
    m.add_wrapped(wrap_pyfunction!(expand_tag))?;
    m.add_wrapped(wrap_pyfunction!(unexpand_tag))?;
    m.add_wrapped(wrap_pyfunction!(increase_version))?;
    m.add_wrapped(wrap_pyfunction!(check_new_revisions))?;
    m.add_wrapped(wrap_pyfunction!(update_version_in_manpage))?;
    m.add_wrapped(wrap_pyfunction!(find_last_version_in_tags))?;
    m.add_wrapped(wrap_pyfunction!(update_version_in_pyproject_toml))?;
    m.add_wrapped(wrap_pyfunction!(pypi_discover_urls))?;
    m.add_wrapped(wrap_pyfunction!(find_name_in_pyproject_toml))?;
    m.add_wrapped(wrap_pyfunction!(pyproject_uses_hatch_vcs))?;
    m.add_wrapped(wrap_pyfunction!(find_hatch_vcs_version))?;
    m.add_wrapped(wrap_pyfunction!(read_project_urls_from_setup_cfg))?;
    m.add_wrapped(wrap_pyfunction!(read_project_urls_from_pyproject_toml))?;
    m.add_wrapped(wrap_pyfunction!(upload_python_artifacts))?;
    m.add_wrapped(wrap_pyfunction!(create_setup_py_artifacts))?;
    m.add_wrapped(wrap_pyfunction!(create_python_artifacts))?;
    m.add("UploadCommandFailed", py.get_type::<UploadCommandFailed>())?;
    m.add_wrapped(wrap_pyfunction!(check_date))?;
    m.add_wrapped(wrap_pyfunction!(check_version))?;
    Ok(())
}
