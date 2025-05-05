use serde::Deserialize;

use std::fs;

use toml;

const CONFIG_FILE_NAME: &str = "disperse.toml";

/// Example configuration file:
/// ```toml
/// [pypi]
/// username = "jelmer"
///
/// [crates.io]
/// username = "jelmer"
/// ```

#[derive(Debug, serde::Deserialize, Default)]
pub struct Config {
    pub pypi: Option<PypiConfig>,
    #[serde(rename = "crates.io")]
    pub crates_io: Option<CratesIoConfig>,
    pub repositories: Option<RepositoriesConfig>,
}

#[derive(Debug, Deserialize)]
pub struct RepositoriesConfig {
    pub owned: Option<Vec<url::Url>>,
}

#[derive(Debug, Deserialize)]
pub struct PypiConfig {
    pub username: String,
}

#[derive(Debug, Deserialize)]
pub struct CratesIoConfig {
    pub username: String,
}

pub fn load_config() -> Result<Option<Config>, Box<dyn std::error::Error>> {
    let xdg = xdg::BaseDirectories::with_prefix("disperse");

    let config_file_path = xdg.place_config_file(CONFIG_FILE_NAME)?;

    // Check if the file exists
    if !config_file_path.exists() {
        return Ok(None);
    }

    // Read the file and parse the TOML
    let contents = fs::read_to_string(config_file_path)?;
    let config: Config = toml::from_str(&contents)?;

    Ok(Some(config))
}
