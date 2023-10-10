use breezyshim::tree::{Tree, WorkingTree};
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::io::Write;
use std::path::Path;
use std::process::Command;

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
    let mut cargo_toml = std::fs::File::open(tree.abspath(Path::new("Cargo.toml"))?)?;
    let mut cargo_toml_contents = String::new();
    cargo_toml.read_to_string(&mut cargo_toml_contents)?;

    // Parse Cargo.toml as TOML
    let mut parsed_toml: toml_edit::Document = cargo_toml_contents.as_str().parse()?;

    // Update the version field
    if let Some(package) = parsed_toml.as_table_mut().get_mut("package") {
        if let Some(version) = package.as_table_mut().and_then(|t| t.get_mut("version")) {
            *version = toml_edit::value(new_version);
        }
    }

    // Serialize the updated TOML back to a string
    let updated_cargo_toml = parsed_toml.to_string();

    // Write the updated TOML back to Cargo.toml
    let mut updated_cargo_toml_file = File::create(tree.abspath(Path::new("Cargo.toml"))?)?;
    updated_cargo_toml_file.write_all(updated_cargo_toml.as_bytes())?;

    // Run 'cargo update' to update dependencies
    Command::new("cargo")
        .arg("update")
        .current_dir(tree.abspath(Path::new("Cargo.toml"))?)
        .spawn()?
        .wait()?;

    Ok(())
}

// Define a function to find the version in the Cargo.toml file
pub fn find_version(tree: &dyn Tree) -> Result<String, Box<dyn Error>> {
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

    Ok(version)
}
