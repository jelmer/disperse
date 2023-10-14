use breezyshim::github::retrieve_github_token;
use log::{debug, error, info};
use octocrab::params::repos::Commitish;
use octocrab::Octocrab;
use std::time::Duration;
use url::Url;

const DEFAULT_GITHUB_CI_TIMEOUT: u64 = 60 * 24;

pub fn init_github() -> Result<Octocrab, Box<dyn std::error::Error>> {
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
    repo_url: &str,
) -> Result<octocrab::models::Repository, Box<dyn std::error::Error>> {
    // Remove ".git" from the end of the URL, if present
    let repo_url = repo_url.strip_suffix(".git").unwrap_or(repo_url);

    let parsed_url = Url::parse(repo_url)?;

    // Extract the owner and repo name from the URL
    let path_segments: Vec<&str> = parsed_url.path_segments().unwrap().collect();
    let owner = path_segments[0];
    let repo_name = path_segments[1];

    // Retrieve the GitHub token using octocrab
    info!("Finding project {}/{} on GitHub", owner, repo_name);

    // Get the repository using octocrab
    Ok(instance.repos(owner, repo_name).get().await?)
}

#[derive(Debug)]
struct GitHubStatusFailed {
    sha: String,
    html_url: Option<String>,
}

impl std::fmt::Display for GitHubStatusPending {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "(GitHubStatusPending) SHA: {}, URL: {}",
            self.sha,
            self.html_url.as_ref().unwrap_or(&"None".to_string())
        )
    }
}

impl std::error::Error for GitHubStatusPending {}

#[derive(Debug)]
struct GitHubStatusPending {
    sha: String,
    html_url: Option<String>,
}

impl std::fmt::Display for GitHubStatusFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "(GitHubStatusFailed) SHA: {}, URL: {}",
            self.sha,
            self.html_url.as_ref().unwrap_or(&"None".to_string())
        )
    }
}

impl std::error::Error for GitHubStatusFailed {}

pub async fn check_gh_repo_action_status(
    instance: &Octocrab,
    repo: octocrab::models::Repository,
    committish: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
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
                let error_msg = format!(
                    "GitHub Status Failed: SHA {}, URL {}",
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                error!("{}", error_msg);
                return Err(Box::new(GitHubStatusFailed {
                    sha: check.head_sha,
                    html_url: check.html_url,
                }));
            }
            None => {
                let error_msg = format!(
                    "GitHub Status Pending: SHA {}, URL {}",
                    check.head_sha,
                    check.html_url.as_ref().unwrap_or(&"None".to_string())
                );
                error!("{}", error_msg);
                return Err(Box::new(GitHubStatusPending {
                    sha: check.head_sha,
                    html_url: check.html_url.clone(),
                }));
            }
        }
    }

    Ok(())
}

pub async fn wait_for_gh_actions(
    instance: &Octocrab,
    repo: octocrab::models::Repository,
    committish: Option<&str>,
    timeout: Option<u64>,
) -> Result<(), Box<dyn std::error::Error>> {
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
                    return Err(Box::new(GitHubStatusFailed {
                        sha: check.head_sha,
                        html_url: check.html_url,
                    }));
                }
                None => {
                    let error_msg = format!(
                        "GitHub Status Pending: SHA {}, URL {}",
                        check.head_sha,
                        check.html_url.as_ref().unwrap_or(&"None".to_string())
                    );
                    error!("{}", error_msg);
                    return Err(Box::new(GitHubStatusPending {
                        sha: check.head_sha,
                        html_url: check.html_url,
                    }));
                }
            }
        }
    }

    Err(Box::new(std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "Timed out waiting for CI",
    )))
}

pub async fn create_github_release(
    instance: &Octocrab,
    repo: octocrab::models::Repository,
    tag_name: &str,
    version: &str,
    description: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("Creating release on GitHub");
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
}
