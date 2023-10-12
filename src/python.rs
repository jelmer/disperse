use crate::Version;
use breezyshim::tree::{Tree, WorkingTree};
use reqwest::header;
use serde_json::Value;

use std::error::Error;

use std::path::Path;

use std::process::Command;
use url::Url;
use xmlrpc::Request;

pub fn update_version_in_pyproject_toml(
    tree: &WorkingTree,
    new_version: &crate::Version,
) -> Result<bool, Box<dyn Error>> {
    let cargo_toml_contents = tree.get_file_text(Path::new("pyproject.toml"))?;

    let mut parsed_toml: toml_edit::Document =
        String::from_utf8_lossy(cargo_toml_contents.as_slice()).parse()?;

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

        project["version"] = toml_edit::value(&new_version.0);
    }

    Ok(true)
}

pub fn find_version_in_pyproject_toml(tree: &dyn Tree) -> Option<Version> {
    let content = tree.get_file_text(Path::new("pyproject.toml")).ok()?;

    let parsed_toml: toml_edit::Document =
        String::from_utf8_lossy(content.as_slice()).parse().ok()?;

    parsed_toml
        .as_table()
        .get("project")
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_str())
        .map(|v| Version(v.to_string()))
}

pub fn pypi_discover_urls(pypi_user: &str) -> Result<Vec<url::Url>, Box<dyn std::error::Error>> {
    let request = Request::new("user_packages").arg(pypi_user);

    let response = request.call_url("https://pypi.org/pypi")?;

    let mut ret = vec![];

    let client = reqwest::blocking::ClientBuilder::new()
        .user_agent(crate::USER_AGENT)
        .build()?;

    for package in response.as_array().unwrap().iter() {
        let package_str = package.as_array().unwrap()[1].as_str().unwrap();

        let req_url = format!("https://pypi.org/pypi/{}/json", package_str);
        let resp = client.get(&req_url).send()?;

        let data: Value = resp.json()?;
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
                    ret.push(url.as_str().unwrap().parse()?);
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
                    ret.push(url.as_str().unwrap().parse()?);
                    break;
                }
            }
        }
    }

    Ok(ret)
}

pub fn pyproject_uses_hatch_vcs(tree: &dyn Tree) -> Result<bool, Box<dyn std::error::Error>> {
    let content = match tree.get_file_text(Path::new("pyproject.toml")) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };

    let parsed_toml: toml_edit::Document = String::from_utf8_lossy(content.as_slice()).parse()?;

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

    let parsed_toml: toml_edit::Document =
        String::from_utf8_lossy(content.as_slice()).parse().ok()?;

    parsed_toml
        .as_table()
        .get("project")
        .and_then(|v| v.as_table())
        .and_then(|v| v.get("name"))
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
}

pub fn find_hatch_vcs_version(tree: &WorkingTree) -> Option<Version> {
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

    Some(Version(
        String::from_utf8_lossy(&output.stdout)
            .split('.')
            .take(3)
            .collect::<Vec<_>>()
            .join("."),
    ))
}

pub fn read_project_urls_from_pyproject_toml(
    path: &std::path::Path,
) -> Result<Vec<(url::Url, Option<String>)>, Box<dyn std::error::Error>> {
    let content = std::fs::read(path)?;

    let parsed_toml: toml_edit::Document = String::from_utf8_lossy(&content).parse()?;

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
    pyo3::Python::with_gil(|py| {
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
    local_tree: &WorkingTree,
    pypi_paths: &[&str],
) -> Result<(), UploadCommandFailed> {
    let mut command = vec!["twine", "upload", "--non-interactive"];
    command.extend(pypi_paths);

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
    local_tree: &WorkingTree,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    pyo3::Python::with_gil(|py| {
        // Initialize an empty vector to store pypi_paths
        let mut pypi_paths: Vec<String> = Vec::new();

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
        let is_pure = !result.call_method0("has_c_libraries")?.extract::<bool>()?
            && !result.call_method0("has_ext_modules")?.extract::<bool>()?;

        let builder = py
            .import("build")?
            .call_method1("ProjectBuilder", (setup_dir,))?;

        if is_pure {
            let wheels = builder.call_method1(
                "build",
                ("wheel", local_tree.abspath(Path::new("dist")).unwrap()),
            )?;
            pypi_paths.push(wheels.extract::<String>()?);
        } else {
            log::warn!("python module is not pure; not uploading binary wheels");
        }

        let sdist_path = builder.call_method1(
            "build",
            ("sdist", local_tree.abspath(Path::new("dist")).unwrap()),
        )?;
        pypi_paths.push(sdist_path.extract::<String>()?);

        Ok(pypi_paths)
    })
}

pub fn create_python_artifacts(
    local_tree: &WorkingTree,
) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
    pyo3::Python::with_gil(|py| {
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
