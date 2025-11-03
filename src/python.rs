use crate::Version;
use breezyshim::error::Error as BrzError;
use breezyshim::tree::{Tree, WorkingTree};
use pyo3::prelude::*;

use serde_json::Value;
use std::path::Path;
use std::process::Command;
use std::str::FromStr;
use url::Url;
use xmlrpc::Request;

#[derive(Debug)]
pub enum Error {
    BrzError(BrzError),
    VersionError(String),
    IoError(std::io::Error),
    Other(String),
}

impl From<BrzError> for Error {
    fn from(e: BrzError) -> Self {
        Error::BrzError(e)
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::IoError(e)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match &self {
            Error::BrzError(e) => write!(f, "Tree error: {}", e),
            Error::VersionError(e) => write!(f, "Version error: {}", e),
            Error::Other(e) => write!(f, "Other error: {}", e),
            Error::IoError(e) => write!(f, "IO error: {}", e),
        }
    }
}

impl std::error::Error for Error {}

pub fn update_version_in_pyproject_toml(
    tree: &dyn WorkingTree,
    new_version: &crate::Version,
) -> Result<bool, Error> {
    let cargo_toml_contents = tree.get_file_text(Path::new("pyproject.toml"))?;

    let mut parsed_toml: toml_edit::DocumentMut = String::from_utf8(cargo_toml_contents)
        .map_err(|e| Error::Other(format!("Invalid UTF-8 in pyproject.toml: {}", e)))?
        .parse()
        .map_err(|e| Error::Other(format!("Invalid TOML in pyproject.toml: {}", e)))?;

    if let Some(project) = parsed_toml
        .as_table_mut()
        .get_mut("project")
        .and_then(|v| v.as_table_mut())
    {
        if let Some(dynamic) = project.get("dynamic").and_then(|v| v.as_array()) {
            if dynamic.iter().any(|v| v.as_str() == Some("version")) {
                return Ok(false);
            }
        }

        if !project.contains_key("version") {
            log::warn!("No version in pyproject.toml");
            return Ok(false);
        }

        project["version"] = toml_edit::value(new_version.to_string());
    }

    Ok(true)
}

pub fn find_version_in_pyproject_toml(tree: &dyn Tree) -> Result<Option<Version>, Error> {
    let content = tree.get_file_text(Path::new("pyproject.toml"))?;

    let parsed_toml: toml_edit::DocumentMut = String::from_utf8(content)
        .map_err(|e| Error::Other(format!("{}", e)))?
        .parse()
        .map_err(|e| Error::Other(format!("Unable to parse TOML: {}", e)))?;

    parsed_toml
        .as_table()
        .get("project")
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_str())
        .map(|v| Version::from_str(v).map_err(Error::VersionError))
        .transpose()
}

pub async fn pypi_discover_urls(pypi_user: &str) -> Result<Vec<url::Url>, Error> {
    let pypi_user = pypi_user.to_string();
    let response = tokio::task::spawn_blocking(move || {
        let request = Request::new("user_packages").arg(pypi_user);
        request.call_url("https://pypi.org/pypi")
    })
    .await
    .map_err(|e| Error::Other(format!("Error joining task: {}", e)))?
    .map_err(|e| Error::Other(format!("Error calling PyPI: {}", e)))?;

    let mut ret = vec![];

    let client = reqwest::ClientBuilder::new()
        .user_agent(crate::USER_AGENT)
        .build()
        .map_err(|e| Error::Other(format!("Error building HTTP client: {}", e)))?;

    for package in response.as_array().unwrap().iter() {
        let package_str = package.as_array().unwrap()[1].as_str().unwrap();

        let req_url = format!("https://pypi.org/pypi/{}/json", package_str);
        let resp = client
            .get(&req_url)
            .send()
            .await
            .map_err(|e| Error::Other(format!("Error fetching {}: {}", req_url, e)))?;

        let data: Value = resp
            .json()
            .await
            .map_err(|e| Error::Other(format!("Error parsing JSON from {}: {}", req_url, e)))?;
        if let Some(project_urls) = data["info"]["project_urls"].as_object() {
            if project_urls.is_empty() {
                log::debug!("Project {} does not have project URLs", package_str);
                continue;
            }

            for (key, url) in project_urls.iter() {
                if url == "UNKNOWN" {
                    continue;
                }
                if key == "Repository" {
                    ret.push(
                        url.as_str().unwrap().parse().map_err(|e| {
                            Error::Other(format!("Error parsing URL {}: {}", url, e))
                        })?,
                    );
                    break;
                }
                let parsed_url = match Url::parse(url.as_str().unwrap()) {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!("Could not parse URL {}: {}", url, e);
                        continue;
                    }
                };
                if parsed_url.host_str() == Some("github.com")
                    && parsed_url.path().trim_matches('/').matches('/').count() == 1
                {
                    ret.push(
                        url.as_str().unwrap().parse().map_err(|e| {
                            Error::Other(format!("Error parsing URL {}: {}", url, e))
                        })?,
                    );
                    break;
                }
            }
        }
    }

    Ok(ret)
}

pub fn pyproject_uses_hatch_vcs(tree: &dyn Tree) -> Result<bool, Error> {
    let content = match tree.get_file_text(Path::new("pyproject.toml")) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    let parsed_toml: toml_edit::DocumentMut = String::from_utf8(content)
        .map_err(|e| Error::Other(format!("Invalid UTF-8 in pyproject.toml: {}", e)))?
        .parse()
        .map_err(|e| Error::Other(format!("Invalid TOML in pyproject.toml: {}", e)))?;

    Ok(parsed_toml
        .as_table()
        .get("tool")
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("hatch"))
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("source"))
        .and_then(|v| v.as_str())
        == Some("vcs"))
}

pub fn find_name_in_pyproject_toml(tree: &dyn Tree) -> Option<String> {
    let content = tree.get_file_text(Path::new("pyproject.toml")).ok()?;

    let parsed_toml: toml_edit::DocumentMut =
        String::from_utf8_lossy(content.as_slice()).parse().ok()?;

    parsed_toml
        .as_table()
        .get("project")
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

pub fn find_hatch_vcs_version(tree: &dyn WorkingTree) -> Option<Version> {
    let cwd = tree.abspath(Path::new(".")).unwrap();

    // run "hatchling version"
    let output = std::process::Command::new("hatchling")
        .arg("version")
        .current_dir(&cwd)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let output = String::from_utf8(output.stdout).unwrap();

    let parts = output.split('.').take(3).collect::<Vec<_>>();

    Some(Version {
        major: parts[0].parse().unwrap(),
        minor: parts.get(1).map(|v| v.parse().unwrap()),
        micro: parts.get(2).map(|v| v.parse().unwrap()),
    })
}

pub fn read_project_urls_from_pyproject_toml(
    path: &std::path::Path,
) -> Result<Vec<(url::Url, Option<String>)>, Error> {
    let content = std::fs::read(path)?;

    let parsed_toml: toml_edit::DocumentMut = String::from_utf8(content)
        .map_err(|e| Error::Other(format!("Invalid UTF-8 in pyproject.toml: {}", e)))?
        .parse()
        .map_err(|e| Error::Other(format!("Invalid TOML in pyproject.toml: {}", e)))?;

    let project_urls = match parsed_toml
        .as_table()
        .get("project")
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("urls"))
        .and_then(|v| v.as_table())
    {
        Some(v) => v,
        None => return Ok(vec![]),
    };

    let mut result = vec![];
    for key in &["GitHub", "Source Code", "Repository"] {
        if let Some(url) = project_urls.get(key).and_then(|v| v.as_str()) {
            if url == "UNKNOWN" {
                continue;
            }
            let parsed_url = match url::Url::parse(url) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("Could not parse URL {}: {}", url, e);
                    continue;
                }
            };
            result.push((parsed_url, None));
        }
    }
    Ok(result)
}

pub fn read_project_urls_from_setup_cfg(
    path: &std::path::Path,
) -> pyo3::PyResult<Vec<(url::Url, Option<String>)>> {
    pyo3::Python::attach(|py| {
        let setuptools = py.import("setuptools.config.setupcfg")?;

        let config = setuptools.call_method1("read_configuration", (path,))?;

        let metadata = match config.get_item("metadata") {
            Ok(v) => v,
            Err(_) => return Ok(vec![]),
        };

        let project_urls = match metadata.get_item("project_urls") {
            Ok(v) => v,
            Err(_) => return Ok(vec![]),
        };

        let mut result = vec![];

        for key in ["GitHub", "Source Code", "Repository"].iter() {
            match project_urls.get_item(key) {
                Ok(url) => {
                    let url_str = url.extract::<String>()?;
                    result.push((url_str.parse::<url::Url>().unwrap(), None));
                }
                Err(_) => continue,
            }
        }

        Ok(result)
    })
}

#[derive(Debug)]
pub struct UploadCommandFailed {
    pub command: Vec<String>,
    pub retcode: Option<i32>,
}

impl std::fmt::Display for UploadCommandFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "`{}` failed", self.command.join(" "))
    }
}

impl std::error::Error for UploadCommandFailed {}

pub fn upload_python_artifacts(
    local_tree: &dyn WorkingTree,
    pypi_paths: &[&std::path::Path],
) -> Result<(), UploadCommandFailed> {
    let mut command = vec!["twine", "upload", "--non-interactive"];
    command.extend(pypi_paths.iter().map(|v| v.to_str().unwrap()));

    let abs_path = local_tree.abspath(Path::new(".")).unwrap();

    let output = Command::new("twine")
        .args(&command[1..])
        .current_dir(&abs_path)
        .status();

    match output {
        Ok(status) => {
            if status.success() {
                Ok(())
            } else {
                Err(UploadCommandFailed {
                    command: command.iter().map(|v| v.to_string()).collect(),
                    retcode: status.code(),
                })
            }
        }
        Err(_) => Err(UploadCommandFailed {
            command: command.iter().map(|v| v.to_string()).collect(),
            retcode: None,
        }),
    }
}

pub fn create_setup_py_artifacts(
    local_tree: &dyn WorkingTree,
) -> pyo3::PyResult<Vec<std::path::PathBuf>> {
    pyo3::Python::attach(|py| {
        // Initialize an empty vector to store pypi_paths
        let mut pypi_paths: Vec<std::path::PathBuf> = Vec::new();

        // Import required Python modules
        let os = py.import("os")?;
        let run_setup = py.import("distutils.core")?.getattr("run_setup")?;
        let _setuptools = py.import("setuptools")?;

        // Save the original directory
        let orig_dir = os.call_method0("getcwd")?;

        // Change to the setup.py directory
        let setup_dir = local_tree.abspath(Path::new(".")).unwrap();
        os.call_method1("chdir", (setup_dir.clone(),))?;

        let result = {
            // Attempt to run setup.py
            let run_setup_result =
                run_setup.call1((local_tree.abspath(Path::new("setup.py")).unwrap(), "config"))?;
            // Change back to the original directory
            os.call_method1("chdir", (orig_dir,))?;

            run_setup_result
        };

        // Check for C libraries and extension modules
        let is_pure = !result
            .call_method0("has_c_libraries")?
            .extract::<Option<bool>>()?
            .unwrap_or(false)
            && !result
                .call_method0("has_ext_modules")?
                .extract::<Option<bool>>()?
                .unwrap_or(false);

        let builder = py
            .import("build")?
            .call_method1("ProjectBuilder", (setup_dir,))?;

        if is_pure {
            let wheels = builder.call_method1(
                "build",
                ("wheel", local_tree.abspath(Path::new("dist")).unwrap()),
            )?;
            pypi_paths.push(wheels.extract::<std::path::PathBuf>()?);
        } else {
            log::warn!("python module is not pure; not uploading binary wheels");
        }

        let sdist_path = builder.call_method1(
            "build",
            ("sdist", local_tree.abspath(Path::new("dist")).unwrap()),
        )?;
        pypi_paths.push(sdist_path.extract::<std::path::PathBuf>()?);

        Ok(pypi_paths)
    })
}

pub fn create_python_artifacts(
    local_tree: &dyn WorkingTree,
) -> pyo3::PyResult<Vec<std::path::PathBuf>> {
    pyo3::Python::attach(|py| {
        let mut pypi_paths = Vec::new();

        let project_builder = py.import("build")?.call_method1(
            "ProjectBuilder",
            (local_tree.abspath(Path::new(".")).unwrap(),),
        )?;

        // Wrap Python exception handling using PyResult
        let wheels = project_builder.call_method1(
            "build",
            ("wheel", local_tree.abspath(Path::new("dist")).unwrap()),
        )?;

        pypi_paths.push(std::path::PathBuf::from(wheels.extract::<String>()?));

        let sdist_path = project_builder.call_method1(
            "build",
            ("source", local_tree.abspath(Path::new("dist")).unwrap()),
        )?;

        pypi_paths.push(std::path::PathBuf::from(sdist_path.extract::<String>()?));

        Ok(pypi_paths)
    })
}
