use breezyshim::tree::{MutableTree, Tree, WorkingTree};
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Command;

pub fn update_version_in_pyproject_toml(
    tree: &WorkingTree,
    new_version: &crate::Version,
) -> Result<bool, Box<dyn Error>> {
    let cargo_toml_contents = tree.get_file_text(Path::new("pyproject.toml"))?;

    // Parse Cargo.toml as TOML
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
