use breezyshim::tree::{MutableTree, Tree, WorkingTree};
use reqwest::header;
use serde_json::Value;
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Command;
use url::Url;
use xmlrpc::{Request, Value as XmlRpcValue};

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

pub fn pypi_discover_urls(pypi_user: &str) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let request = Request::new("user_packages").arg(pypi_user);

    let response = request.call_url("https://pypi.org/pypi")?;

    let mut ret = vec![];

    for package in response.as_array().unwrap().iter() {
        let package_str = package.as_array().unwrap()[1].as_str().unwrap();

        let version_string = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);
        let req_url = format!("https://pypi.org/pypi/{}/json", package_str);
        let client = reqwest::blocking::Client::new();
        let resp = client
            .get(&req_url)
            .header(header::USER_AGENT, format!("disperse/{}", version_string))
            .send()?;

        let data: Value = resp.json()?;
        if let Some(project_urls) = data["info"]["project_urls"].as_object() {
            if project_urls.is_empty() {
                log::debug!("Project {} does not have project URLs", package_str);
                continue;
            }

            for (key, url) in project_urls.iter() {
                if key == "Repository" {
                    ret.push(url.as_str().unwrap().to_string());
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
                    ret.push(url.as_str().unwrap().to_string());
                    break;
                }
            }
        }
    }

    Ok(ret)
}
