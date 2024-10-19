use breezyshim::error::Error as BrzError;
use breezyshim::tree::Tree;
use std::path::{Path, PathBuf};
include!(concat!(env!("OUT_DIR"), "/generated/mod.rs"));

#[derive(serde::Deserialize, Default)]
pub struct ProjectConfig {
    #[serde(default)]
    pub name: Option<String>,

    #[serde(default, rename = "tag-name")]
    pub tag_name: Option<String>,

    #[serde(default)]
    pub update_version: Vec<UpdateVersion>,

    #[serde(default, rename = "update-manpage")]
    pub update_manpages: Vec<PathBuf>,

    #[serde(default)]
    pub launchpad: Option<Launchpad>,

    #[serde(default)]
    pub github: Option<GitHub>,

    #[serde(default, rename = "news-file")]
    pub news_file: Option<PathBuf>,

    #[serde(default, rename = "pre-dist-command")]
    pub pre_dist_command: Option<String>,

    #[serde(default, rename = "verify-command")]
    pub verify_command: Option<String>,

    #[serde(default, rename = "twine-upload")]
    pub twine_upload: Option<bool>,

    #[serde(default, rename = "tarball-location")]
    pub tarball_location: Vec<String>,

    #[serde(default, rename = "release-timeout")]
    pub release_timeout: Option<u64>,

    #[serde(default, rename = "ci-timeout")]
    pub ci_timeout: Option<u64>,
}

#[derive(serde::Deserialize)]
pub struct GitHub {
    pub url: String,
    pub branch: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct Launchpad {
    pub project: String,
    pub series: Option<String>,
}

#[derive(serde::Deserialize)]
pub struct UpdateVersion {
    pub path: std::path::PathBuf,
    pub r#match: Option<String>,
    #[serde(rename = "new-line")]
    pub new_line: String,
}

impl From<config::UpdateVersion> for UpdateVersion {
    fn from(u: config::UpdateVersion) -> Self {
        UpdateVersion {
            path: u.path.unwrap().into(),
            r#match: u.match_,
            new_line: u.new_line.unwrap(),
        }
    }
}

impl From<config::Project> for ProjectConfig {
    fn from(p: config::Project) -> Self {
        ProjectConfig {
            name: p.name,
            update_version: p.update_version.into_iter().map(|u| u.into()).collect(),
            launchpad: p.launchpad_project.as_ref().map(|_l| Launchpad {
                project: p.launchpad_project.clone().unwrap(),
                series: p.launchpad_series.clone(),
            }),
            github: p.github_url.as_ref().map(|_g| GitHub {
                url: p.github_url.clone().unwrap(),
                branch: p.github_branch.clone(),
            }),
            news_file: p.news_file.clone().map(|n| n.into()),
            update_manpages: p.update_manpages.into_iter().map(|u| u.into()).collect(),
            tag_name: p.tag_name.clone(),
            pre_dist_command: p.pre_dist_command.clone(),
            verify_command: p.verify_command.clone(),
            twine_upload: p.skip_twine_upload.map(|t| !t),
            tarball_location: p.tarball_location.clone(),
            release_timeout: p.timeout_days.map(|t| t as u64),
            ci_timeout: p.ci_timeout.map(|t| t as u64),
        }
    }
}

fn read_old_project(f: &mut dyn std::io::Read) -> config::Project {
    let mut s = String::new();
    std::io::Read::read_to_string(f, &mut s).unwrap();

    protobuf::text_format::parse_from_str(&s).unwrap()
}

fn read_toml_project(f: &mut dyn std::io::Read) -> ProjectConfig {
    let mut s = String::new();
    std::io::Read::read_to_string(f, &mut s).unwrap();

    let ret: ProjectConfig = toml::from_str(&s).unwrap();
    ret
}

pub fn read_project_with_fallback(tree: &dyn Tree) -> Result<ProjectConfig, BrzError> {
    let mut first_error = None;

    match tree.get_file(Path::new("disperse.toml")) {
        Ok(mut f) => return Ok(read_toml_project(&mut f)),
        Err(e @ BrzError::NoSuchFile(_)) => {
            first_error = Some(e);
        }
        Err(e) => {
            return Err(e);
        }
    }

    let mut old_config = match tree.get_file(Path::new("disperse.conf")) {
        Ok(f) => f,
        Err(BrzError::NoSuchFile(_)) => match tree.get_file(Path::new("releaser.conf")) {
            Err(BrzError::NoSuchFile(_)) => {
                return Err(first_error.unwrap());
            }
            Err(e) => return Err(e),
            Ok(f) => f,
        },
        Err(e) => return Err(e),
    };

    Ok(read_old_project(&mut old_config).into())
}
