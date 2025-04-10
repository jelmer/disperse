use breezyshim::github::retrieve_github_token;
use log::{debug, error, info};
use octocrab::params::repos::Commitish;
use octocrab::Octocrab;
use std::time::Duration;
use url::Url;

const DEFAULT_GITHUB_CI_TIMEOUT: u64 = 60 * 24;

#[derive(Debug)]
pub enum Error {
    InvalidGitHubUrl(String, String),
    GitHubError(octocrab::Error),
    TimedOut,
}

impl From<octocrab::Error> for Error {
    fn from(err: octocrab::Error) -> Self {
        Error::GitHubError(err)
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Error::InvalidGitHubUrl(url, msg) => {
                write!(f, "Invalid GitHub URL {}: {}", url, msg)
            }
            Error::GitHubError(err) => write!(f, "GitHub Error: {}", err),
            Error::TimedOut => write!(f, "Timed out waiting for GitHub"),
        }
    }
}

impl std::error::Error for Error {}

pub enum GitHubCIStatus {
    Ok,
    Failed {
        sha: String,
        html_url: Option<String>,
    },
    Pending {
        sha: String,
        html_url: Option<String>,
    },
}

impl GitHubCIStatus {
    pub fn is_ok(&self) -> bool {
        matches!(self, GitHubCIStatus::Ok)
    }
}

impl std::fmt::Display for GitHubCIStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            GitHubCIStatus::Ok => write!(f, "GitHub CI Status: OK"),
            GitHubCIStatus::Failed {
                sha,
                html_url: Some(url),
            } => write!(f, "GitHub CI Status: Failed: SHA {}, URL {}", sha, url),
            GitHubCIStatus::Failed {
                sha,
                html_url: None,
            } => write!(f, "GitHub CI Status: Failed: SHA {}, URL None", sha),
            GitHubCIStatus::Pending {
                sha,
                html_url: Some(url),
            } => write!(f, "GitHub CI Status: Pending: SHA {}, URL {}", sha, url),
            GitHubCIStatus::Pending {
                sha,
                html_url: None,
            } => write!(f, "GitHub CI Status: Pending: SHA {}, URL None", sha),
        }
    }
}

pub fn init_github() -> Result<Octocrab, Error> {
    let github_token = match std::env::var("GITHUB_TOKEN") {
        Ok(token) => token,
        Err(_) => {
            debug!("GITHUB_TOKEN environment variable not set");
            retrieve_github_token()
        }
    };

    let instance = Octocrab::builder().personal_token(github_token).build()?;

    Ok(instance)
}

pub async fn get_github_repo(
    instance: &Octocrab,
    repo_url: &url::Url,
) -> Result<octocrab::models::Repository, Error> {
    // Remove ".git" from the end of the URL, if present
    let repo_url = repo_url.as_str();
    let repo_url = repo_url.strip_suffix(".git").unwrap_or(repo_url);

    let parsed_url = Url::parse(repo_url)
        .map_err(|_| Error::InvalidGitHubUrl(repo_url.to_string(), "Invalid URL".to_string()))?;

    let parsed_url = crate::drop_segment_parameters(&parsed_url);

    // Extract the owner and repo name from the URL
    let path_segments: Vec<&str> = parsed_url.path_segments().unwrap().collect();
    let owner = path_segments[0];
    let repo_name = path_segments[1];

    // Retrieve the GitHub token using octocrab
    info!("Finding project {}/{} on GitHub", owner, repo_name);

    // Get the repository using octocrab
    Ok(instance.repos(owner, repo_name).get().await?)
}

pub async fn check_gh_repo_action_status(
    instance: &Octocrab,
    repo: &octocrab::models::Repository,
    committish: Option<&str>,
) -> Result<GitHubCIStatus, Error> {
    let committish = committish.unwrap_or("HEAD");

    let commit = instance
        .commits(&repo.owner.as_ref().unwrap().login, &repo.name)
        .get(committish)
        .await?;

    for check in instance
        .checks(&repo.owner.as_ref().unwrap().login, &repo.name)
        .list_check_runs_for_git_ref(Commitish(commit.sha.clone()))
        .send()
        .await?
        .check_runs
    {
        match check.conclusion.as_deref() {
            Some("success") | Some("skipped") => continue,
            Some(_) => {
                error!(
                    "GitHub Status Failed: SHA {}, URL {}",
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                return Ok(GitHubCIStatus::Failed {
                    sha: check.head_sha,
                    html_url: check.html_url,
                });
            }
            None => {
                error!(
                    "GitHub Status Pending: SHA {}, URL {}",
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                return Ok(GitHubCIStatus::Pending {
                    sha: check.head_sha,
                    html_url: check.html_url.clone(),
                });
            }
        }
    }

    Ok(GitHubCIStatus::Ok)
}

pub async fn wait_for_gh_actions(
    instance: &Octocrab,
    repo: &octocrab::models::Repository,
    committish: Option<&str>,
    timeout: Option<u64>,
) -> Result<GitHubCIStatus, Error> {
    let timeout = timeout.unwrap_or(DEFAULT_GITHUB_CI_TIMEOUT);
    info!(
        "Waiting for CI for {} on {} to go green",
        repo.name,
        committish.unwrap_or("HEAD")
    );
    let committish = committish.unwrap_or("HEAD");

    let commit = instance
        .commits(&repo.owner.as_ref().unwrap().login, &repo.name)
        .get(committish)
        .await?;

    let start_time = std::time::Instant::now();

    while start_time.elapsed().as_secs() < timeout {
        let check_runs = instance
            .checks(&repo.owner.as_ref().unwrap().login, &repo.name)
            .list_check_runs_for_git_ref(Commitish(commit.sha.clone()))
            .send()
            .await?
            .check_runs;

        match summarize_status(check_runs.as_slice()) {
            GitHubCIStatus::Ok => {
                info!("CI for {} on {} is green", repo.name, committish);
                return Ok(GitHubCIStatus::Ok);
            }
            GitHubCIStatus::Pending { .. } => {
                std::thread::sleep(Duration::from_secs(30));
            }
            GitHubCIStatus::Failed { html_url, sha } => {
                return Ok(GitHubCIStatus::Failed { sha, html_url });
            }
        }
    }

    Err(Error::TimedOut)
}

fn summarize_status(check_runs: &[octocrab::models::checks::CheckRun]) -> GitHubCIStatus {
    for check in check_runs {
        match check.conclusion.as_deref() {
            Some("success") | Some("skipped") => {}
            Some("pending") => {
                error!(
                    "GitHub Status Pending: SHA {}, URL {}",
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                return GitHubCIStatus::Pending {
                    sha: check.head_sha.clone(),
                    html_url: check.html_url.clone(),
                };
            }
            Some(e) => {
                error!(
                    "GitHub Status Failed ({}): SHA {}, URL {}",
                    e,
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                return GitHubCIStatus::Failed {
                    sha: check.head_sha.clone(),
                    html_url: check.html_url.clone(),
                };
            }
            None => {
                error!(
                    "GitHub Status Pending: SHA {}, URL {}",
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                return GitHubCIStatus::Pending {
                    sha: check.head_sha.clone(),
                    html_url: check.html_url.clone(),
                };
            }
        }
    }

    GitHubCIStatus::Ok
}

pub async fn create_github_release(
    instance: &Octocrab,
    repo: &octocrab::models::Repository,
    tag_name: &str,
    version: &str,
    description: Option<&str>,
) -> Result<(), Error> {
    info!("Creating release on GitHub");

    instance
        .repos(&repo.owner.as_ref().unwrap().login, &repo.name)
        .releases()
        .create(tag_name)
        .name(version)
        .body(description.unwrap_or(&format!("Release {}.", version)))
        .send()
        .await?;

    Ok(())
}

pub fn login() -> Result<Octocrab, Error> {
    let entry = keyring::Entry::new("github.com", "personal_token").unwrap();
    let token = match std::env::var("GITHUB_TOKEN") {
        Ok(token) => Some(token),
        Err(std::env::VarError::NotPresent) => match entry.get_password() {
            Ok(token) => Some(token),
            Err(keyring::Error::NoEntry) => None,
            Err(e) => {
                log::error!("Unable to read GitHub personal token from keyring: {}", e);
                None
            }
        },
        Err(e) => {
            log::error!(
                "Unable to read GitHub personal token from environment: {}",
                e
            );
            None
        }
    };

    let builder = if let Some(token) = token {
        log::info!("Using GitHub personal token from keyring");
        octocrab::OctocrabBuilder::new().personal_token(token)
    } else {
        println!("Please enter your GitHub personal token");
        let mut personal_token = String::new();
        std::io::stdin().read_line(&mut personal_token).unwrap();
        let personal_token = personal_token.trim();
        entry.set_password(personal_token).unwrap();
        octocrab::OctocrabBuilder::new().personal_token(personal_token.to_string())
    };
    Ok(builder.build()?)
}
