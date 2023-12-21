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
    TimedOut
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
            } => write!(
                f,
                "GitHub CI Status: Failed: SHA {}, URL {}",
                sha,
                url
            ),
            GitHubCIStatus::Failed {
                sha,
                html_url: None,
            } => write!(f, "GitHub CI Status: Failed: SHA {}, URL None", sha),
            GitHubCIStatus::Pending {
                sha,
                html_url: Some(url),
            } => write!(
                f,
                "GitHub CI Status: Pending: SHA {}, URL {}",
                sha,
                url
            ),
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

pub fn get_github_repo(
    instance: &Octocrab,
    repo_url: &url::Url
) -> Result<octocrab::models::Repository, Error> {
    // Remove ".git" from the end of the URL, if present
    let repo_url = repo_url.as_str();
    let repo_url = repo_url.strip_suffix(".git").unwrap_or(repo_url);

    let parsed_url = Url::parse(repo_url)
        .map_err(|_| Error::InvalidGitHubUrl(repo_url.to_string(), "Invalid URL".to_string()))?;

    // Extract the owner and repo name from the URL
    let path_segments: Vec<&str> = parsed_url.path_segments().unwrap().collect();
    let owner = path_segments[0];
    let repo_name = path_segments[1];

    // Retrieve the GitHub token using octocrab
    info!("Finding project {}/{} on GitHub", owner, repo_name);

    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        // Get the repository using octocrab
        Ok(instance.repos(owner, repo_name).get().await?)
    })
}

pub fn check_gh_repo_action_status(
    instance: &Octocrab,
    repo: &octocrab::models::Repository,
    committish: Option<&str>,
) -> Result<GitHubCIStatus, Error> {
    let committish = committish.unwrap_or("HEAD");

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
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
                    let error_msg = format!(
                        "GitHub Status Failed: SHA {}, URL {}",
                        check.head_sha,
                        check.html_url.as_ref().unwrap_or(&"None".to_string())
                    );
                    error!("{}", error_msg);
                    return Ok(GitHubCIStatus::Failed {
                        sha: check.head_sha,
                        html_url: check.html_url,
                    });
                }
                None => {
                    let error_msg = format!(
                        "GitHub Status Pending: SHA {}, URL {}",
                        check.head_sha,
                        check.html_url.as_ref().unwrap_or(&"None".to_string())
                    );
                    error!("{}", error_msg);
                    return Ok(GitHubCIStatus::Pending {
                        sha: check.head_sha,
                        html_url: check.html_url.clone(),
                    });
                }
            }
        }

        Ok(GitHubCIStatus::Ok)
    })
}

pub fn wait_for_gh_actions(
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

    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        let commit = instance
            .commits(&repo.owner.as_ref().unwrap().login, &repo.name)
            .get(committish)
            .await?;

        let start_time = std::time::Instant::now();

        while start_time.elapsed().as_secs() < timeout {
            for check in instance
                .checks(&repo.owner.as_ref().unwrap().login, &repo.name)
                .list_check_runs_for_git_ref(Commitish(commit.sha.clone()))
                .send()
                .await?
                .check_runs
            {
                match check.conclusion.as_deref() {
                    Some("success") | Some("skipped") => continue,
                    Some("pending") => {
                        std::thread::sleep(Duration::from_secs(30));
                        break;
                    }
                    Some(_) => {
                        let error_msg = format!(
                            "GitHub Status Failed: SHA {}, URL {}",
                            check.head_sha,
                            check.html_url.as_ref().unwrap_or(&"None".to_string())
                        );
                        error!("{}", error_msg);
                        return Ok(GitHubCIStatus::Failed {
                            sha: check.head_sha,
                            html_url: check.html_url,
                        });
                    }
                    None => {
                        let error_msg = format!(
                            "GitHub Status Pending: SHA {}, URL {}",
                            check.head_sha,
                            check.html_url.as_ref().unwrap_or(&"None".to_string())
                        );
                        error!("{}", error_msg);
                        return Ok(GitHubCIStatus::Pending {
                            sha: check.head_sha,
                            html_url: check.html_url,
                        });
                    }
                }
            }
        }

        Err(Error::TimedOut)
    })
}

pub fn create_github_release(
    instance: &Octocrab,
    repo: &octocrab::models::Repository,
    tag_name: &str,
    version: &str,
    description: Option<&str>,
) -> Result<(), Error> {
    info!("Creating release on GitHub");

    let rt = tokio::runtime::Runtime::new().unwrap();

    rt.block_on(async {
        instance
            .repos(&repo.owner.as_ref().unwrap().login, &repo.name)
            .releases()
            .create(version)
            .target_commitish(tag_name)
            .name(version)
            .body(description.unwrap_or(&format!("Release {}.", version)))
            .send()
            .await?;

        Ok(())
    })
}
