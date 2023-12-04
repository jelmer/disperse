use breezyshim::tree::{MutableTree, Tree, WorkingTree};
use std::error::Error;

use std::path::Path;
use std::process::Command;

pub fn get_owned_crates(user: &str) -> Result<Vec<url::Url>, Box<dyn Error>> {
    let client =
        crates_io_api::SyncClient::new(crate::USER_AGENT, std::time::Duration::from_millis(1000))?;

    let user = client.user(user)?;

    let query = crates_io_api::CratesQueryBuilder::new().user_id(user.id);

    let owned_crates = client.crates(query.build())?;

    Ok(owned_crates
        .crates
        .into_iter()
        .filter_map(|c| c.repository)
        .map(|r| url::Url::parse(r.as_str()).unwrap())
        .collect::<Vec<url::Url>>())
}

// Define a function to publish a Rust package using Cargo
pub fn publish(tree: &WorkingTree, subpath: &Path) -> Result<(), Box<dyn Error>> {
    Command::new("cargo")
        .arg("publish")
        .current_dir(tree.abspath(subpath)?)
        .spawn()?
        .wait()?;
    Ok(())
}

// Define a function to update the version in the Cargo.toml file
pub fn update_version(tree: &WorkingTree, new_version: &str) -> Result<(), Box<dyn Error>> {
    // Read the Cargo.toml file
    let cargo_toml_contents = tree.get_file_text(Path::new("Cargo.toml"))?;

    // Parse Cargo.toml as TOML
    let mut parsed_toml: toml_edit::Document =
        String::from_utf8_lossy(cargo_toml_contents.as_slice()).parse()?;

    // Update the version field
    if let Some(package) = parsed_toml.as_table_mut().get_mut("package") {
        if let Some(version) = package.as_table_mut().and_then(|t| t.get_mut("version")) {
            *version = toml_edit::value(new_version);
        }
    }

    // Serialize the updated TOML back to a string
    let updated_cargo_toml = parsed_toml.to_string();

    // Write the updated TOML back to Cargo.toml
    tree.put_file_bytes_non_atomic(Path::new("Cargo.toml"), updated_cargo_toml.as_bytes())?;

    // Run 'cargo update' to update dependencies
    Command::new("cargo")
        .arg("update")
        .current_dir(tree.abspath(Path::new("."))?)
        .spawn()?
        .wait()?;

    Ok(())
}

// Define a function to find the version in the Cargo.toml file
pub fn find_version(tree: &dyn Tree) -> Result<crate::version::Version, Box<dyn Error>> {
    // Read the Cargo.toml file
    let cargo_toml_contents = tree.get_file_text(Path::new("Cargo.toml"))?;

    // Parse Cargo.toml as TOML
    let parsed_toml: toml_edit::Document = String::from_utf8(cargo_toml_contents)?.parse()?;

    // Retrieve the version field
    let version = parsed_toml
        .as_table()
        .get("package")
        .and_then(|p| p.as_table())
        .and_then(|t| t.get("version"))
        .and_then(|v| v.as_str())
        .ok_or("Version not found in Cargo.toml")?
        .to_string();

    Ok(version.as_str().parse()?)
}
