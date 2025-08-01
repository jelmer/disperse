use breezyshim::branch::Branch;
use breezyshim::error::Error as BrzError;
use breezyshim::repository::Repository;
use breezyshim::tree::{MutableTree, Tree};
use breezyshim::workingtree::{self, WorkingTree};
use clap::Parser;
use disperse::project_config::{read_project_with_fallback, ProjectConfig};
use disperse::version::Version;
use disperse::{find_last_version_in_files, find_last_version_in_tags};
use maplit::hashmap;
use std::io::Write;
use std::path::Path;
use url::Url;

use prometheus::{
    default_registry, register_int_counter, register_int_counter_vec, Encoder, IntCounter,
    IntCounterVec, TextEncoder,
};

lazy_static::lazy_static! {
    static ref CI_IGNORED_COUNT: IntCounterVec = register_int_counter_vec!(
        "ci_ignored",
        "Number of projects that were ignored because CI is not passing",
        &["project"]
    ).unwrap();

    static ref NO_DISPERSE_CONFIG: IntCounter = register_int_counter!(
        "no_disperse_config",
        "Number of projects that were ignored because they have no disperse configuration",
    ).unwrap();

    static ref NO_UNRELEASED_CHANGES_COUNT: IntCounterVec = register_int_counter_vec!(
        "no_unreleased_changes",
        "There were no unreleased changes",
        &["project"]
    ).unwrap();

    static ref RECENT_COMMITS_COUNT: IntCounterVec = register_int_counter_vec!(
        "recent_commits",
        "There were recent commits, so no release was done",
        &["project"],
    ).unwrap();

    static ref PRE_DIST_COMMAND_FAILED: IntCounterVec = register_int_counter_vec!(
        "pre_dist_command_failed",
        "The pre-dist command failed to run",
        &["project"],
    ).unwrap();

    static ref VERIFY_COMMAND_FAILED: IntCounterVec = register_int_counter_vec!(
        "verify_command_failed",
        "The verify command failed to run",
        &["project"],
    ).unwrap();

    static ref BRANCH_PROTECTED_COUNT: IntCounterVec = register_int_counter_vec!(
        "branch_protected",
        "The branch was protected",
        &["project"]
    ).unwrap();

    static ref RELEASED_COUNT: IntCounterVec = register_int_counter_vec!(
        "released",
        "Released projects",
        &["project"]
    ).unwrap();

    static ref RELEASE_TAG_EXISTS: IntCounterVec = register_int_counter_vec!(
        "release_tag_exists",
        "A release tag already exists",
        &["project"]).unwrap();
}

async fn push_to_gateway(prometheus_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    encoder.encode(&default_registry().gather(), &mut buffer)?;

    let metrics = String::from_utf8(buffer)?;

    let url = format!("{}/metrics/job/disperse", prometheus_url);
    reqwest::Client::new()
        .post(url)
        .body(metrics)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

#[derive(Parser)]
struct Args {
    /// Print debug output
    #[clap(long)]
    debug: bool,

    /// Do not actually do anything
    #[clap(long)]
    dry_run: bool,

    /// Prometheus push gateway URL
    #[clap(long)]
    prometheus: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Release a new version of a project
    Release(ReleaseArgs),

    /// Discover projects that need to be released
    Discover(DiscoverArgs),

    /// Validate disperse configuration
    Validate(ValidateArgs),

    /// Show information about a project
    Info(InfoArgs),

    /// Run the verify command
    Verify(VerifyArgs),

    /// Migrate configuration to a new version
    Migrate(MigrateArgs),

    /// Initialize a new project
    Init(InitArgs),
}

#[derive(clap::Args)]
struct VerifyArgs {
    /// Path or URL for project
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

#[derive(clap::Args)]
struct ReleaseArgs {
    #[clap(default_value = ".")]
    url: Vec<String>,

    /// New version to release
    #[clap(long)]
    new_version: Option<String>,

    /// Release even if the CI is not passing
    #[clap(long)]
    ignore_ci: bool,

    /// Release even if the verify_command fails
    #[clap(long)]
    ignore_verify_command: bool,

    #[clap(long)]
    discover: bool,

    #[clap(long)]
    /// Preserve the temporary directory used for building
    preserve_temp: bool,
}

#[derive(clap::Args)]
struct DiscoverArgs {
    /// Pypi users to upload for
    #[clap(long, env = "PYPI_USERNAME")]
    pypi_user: Vec<String>,

    /// Crates.io users to upload for
    #[clap(long, env = "CRATES_IO_USERNAME")]
    crates_io_user: Option<String>,

    /// Force a new release, even if timeout is not reached
    #[clap(long)]
    force: bool,

    /// Display status only, do not create new releases
    #[clap(long)]
    info: bool,

    /// Just display URLs
    #[clap(long, conflicts_with = "info")]
    urls: bool,

    /// Do not exit with non-zero if projects failed to be released
    #[clap(long)]
    r#try: bool,
}

#[derive(clap::Args)]
struct ValidateArgs {
    /// Path or URL for project
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

#[derive(clap::Args)]
struct InfoArgs {
    /// Path or URL for project
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

#[derive(clap::Args)]
struct MigrateArgs {
    /// Path or URL for project
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

#[derive(clap::Args)]
struct InitArgs {
    /// Path or URL for project
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

pub fn find_last_version(
    workingtree: &dyn WorkingTree,
    cfg: &ProjectConfig,
) -> Result<(Option<Version>, Option<disperse::Status>), Box<dyn std::error::Error>> {
    match find_last_version_in_files(workingtree, cfg) {
        Ok(Some((v, s))) => {
            return Ok((Some(v), s));
        }
        Ok(Option::None) => {
            log::debug!("No version found in files");
        }
        Err(e) => {
            log::info!("Error finding last version in files: {}", e);
        }
    }

    if let Some(tag_name) = cfg.tag_name.as_deref() {
        match find_last_version_in_tags(&workingtree.branch(), tag_name) {
            Ok((Some(v), s)) => {
                return Ok((Some(v), s));
            }
            Ok((Option::None, _)) => {
                log::debug!("No version found in tags");
            }
            Err(e) => {
                log::info!("Error finding last version in tags: {}", e);
            }
        }
    }

    Ok((None, None))
}

pub fn info(tree: &dyn WorkingTree, branch: &dyn breezyshim::branch::Branch) -> i32 {
    let cfg = match disperse::project_config::read_project_with_fallback(tree) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::info!("Error loading configuration: {}", e);
            return 1;
        }
    };

    let name = if let Some(name) = cfg.name.as_ref() {
        Some(name.clone())
    } else if tree.has_filename(Path::new("pyproject.toml")) {
        disperse::python::find_name_in_pyproject_toml(tree)
    } else {
        None
    };

    if let Some(name) = name {
        log::info!("Project: {}", name);
    }

    let (mut last_version, last_version_status) = match find_last_version(tree, &cfg) {
        Ok((Some(v), s)) => (v, s),
        Ok((Option::None, _)) => {
            log::info!("No version found");
            return 1;
        }
        Err(e) => {
            log::info!("Error loading last version: {}", e);
            return 1;
        }
    };

    log::info!("Last release: {}", last_version.to_string());
    if let Some(status) = last_version_status {
        log::info!("  status: {}", status.to_string());
    }

    let tags = branch.tags().unwrap();

    let tag_name = disperse::version::expand_tag(cfg.tag_name.as_deref().unwrap(), &last_version);
    match tags.lookup_tag(tag_name.as_str()) {
        Ok(release_revid) => {
            log::info!("  tag name: {} ({})", tag_name, release_revid);

            let rev = branch.repository().get_revision(&release_revid).unwrap();
            log::info!("  date: {}", rev.datetime().format("%Y-%m-%d %H:%M:%S"));

            if rev.revision_id != branch.last_revision() {
                let graph = branch.repository().get_graph();
                let missing = match graph
                    .iter_lefthand_ancestry(&branch.last_revision(), Some(&[release_revid]))
                {
                    Ok(iter) => iter.collect::<Result<Vec<_>, _>>().unwrap(),
                    Err(e) => {
                        log::error!("Failed to get ancestry: {}", e);
                        return 1;
                    }
                };
                if missing.last().map(|r| r.is_null()).unwrap() {
                    log::info!("  last release not found in ancestry");
                } else {
                    let first = branch
                        .repository()
                        .get_revision(missing.last().unwrap())
                        .unwrap();
                    let first_timestamp = first.datetime();
                    let first_age = chrono::Utc::now()
                        .signed_duration_since(first_timestamp)
                        .num_days();
                    log::info!(
                        "  {} revisions since last release. First is {} days old.",
                        missing.len(),
                        first_age,
                    );
                }
            } else {
                log::info!("  no revisions since last release");
            }
        }
        Err(BrzError::NoSuchTag(name)) => {
            log::info!("  tag {} for previous release not found", name);
        }
        Err(BrzError::TagAlreadyExists(_name)) => {
            unreachable!();
        }
        Err(e) => {
            log::info!("  error loading tag: {}", e);
        }
    };

    match disperse::find_pending_version(tree, &cfg) {
        Ok(new_version) => {
            log::info!("Pending version: {}", new_version.to_string());
            0
        }
        Err(disperse::FindPendingVersionError::OddPendingVersion(e)) => {
            log::info!("Pending version: {} (odd)", e);
            1
        }
        Err(disperse::FindPendingVersionError::NotFound) => {
            disperse::version::increase_version(&mut last_version, -1);
            log::info!(
                "No pending version found; would use {}",
                last_version.to_string()
            );
            0
        }
        Err(disperse::FindPendingVersionError::NoUnreleasedChanges) => {
            log::info!("No unreleased changes");
            0
        }
        Err(disperse::FindPendingVersionError::Other(e)) => {
            log::info!("Error finding pending version: {}", e);
            1
        }
    }
}

/// Print information about the current project.
fn info_many(urls: &[Url]) -> i32 {
    let mut ret = 0;

    for url in urls {
        if url.to_string() != "." {
            log::info!("Processing {}", url);
        }

        let (local_wt, branch) = match breezyshim::controldir::open_tree_or_branch(url, None, None)
        {
            Ok(x) => x,
            Err(e) => {
                ret = 1;
                log::error!("Unable to open {}: {}", url, e);
                continue;
            }
        };

        if let Some(wt) = local_wt {
            let lock = wt.lock_read();
            ret += info(&wt, &*branch);
            std::mem::drop(lock);
        } else {
            let main_branch_box = breezyshim::branch::open(url).unwrap();
            // Downcast Box<dyn Branch> to GenericBranch
            let main_branch = main_branch_box
                .as_any()
                .downcast_ref::<breezyshim::branch::GenericBranch>()
                .expect("Expected GenericBranch")
                .clone();
            let ws = silver_platter::workspace::Workspace::builder()
                .main_branch(main_branch)
                .build()
                .unwrap();
            let lock = ws.local_tree().lock_read();
            let r = info(ws.local_tree(), &ws.local_tree().branch());
            std::mem::drop(lock);
            ret += r;
        }
    }
    ret
}

pub fn pick_new_version(tree: &dyn WorkingTree, cfg: &ProjectConfig) -> Result<Version, String> {
    match disperse::find_pending_version(tree, cfg) {
        Ok(new_version) => {
            return Ok(new_version);
        }
        Err(disperse::FindPendingVersionError::NotFound) => {}
        Err(disperse::FindPendingVersionError::OddPendingVersion(e)) => {
            return Err(format!("Pending version: {} (odd)", e));
        }
        Err(disperse::FindPendingVersionError::NoUnreleasedChanges) => {
            return Err("No unreleased changes".to_string());
        }
        Err(disperse::FindPendingVersionError::Other(o)) => {
            return Err(format!("Error finding pending version: {}", o));
        }
    }

    let mut last_version = match find_last_version(tree, cfg) {
        Ok((Some(v), _)) => v,
        Ok((Option::None, _)) => {
            return Err("No version found".to_string());
        }
        Err(e) => {
            return Err(format!("Error loading last version: {}", e));
        }
    };
    let tags = tree.branch().tags().unwrap();
    loop {
        let last_version_tag_name =
            disperse::version::expand_tag(cfg.tag_name.as_ref().unwrap(), &last_version);
        if !tags.has_tag(last_version_tag_name.as_str()) {
            break;
        }
        disperse::version::increase_version(&mut last_version, -1);
    }
    Ok(last_version)
}

#[derive(Debug)]
pub enum ReleaseError {
    /// The repository is unavailable.
    RepositoryUnavailable {
        url: String,
        reason: String,
    },

    /// There are no changes since the last release.
    NoUnreleasedChanges,

    NoVersion,

    /// The pending version is not parseable.
    OddPendingVersion {
        version: String,
    },

    NoSuchTag,
    NoDisperseConfig,
    PreDistCommandFailed {
        command: String,
        status: Option<std::process::ExitStatus>,
    },
    UploadCommandFailed {
        command: String,
        status: Option<std::process::ExitStatus>,
        reason: Option<String>,
    },
    VerifyCommandFailed {
        command: String,
        status: Option<std::process::ExitStatus>,
    },
    ReleaseTagExists {
        project: String,
        tag: String,
        version: Version,
    },
    CommitFailed(String),
    RecentCommits {
        min_commit_age: i64,
        commit_age: i64,
    },
    CreateTagFailed {
        tag_name: String,
        status: Option<std::process::ExitStatus>,
        reason: Option<String>,
    },
    CIFailed(String),
    CIPending(String),
    PublishArtifactsFailed(String),
    DistCreationFailed,
    NoPublicBranch,
    Other(String),
}

impl From<silver_platter::workspace::Error> for ReleaseError {
    fn from(_e: silver_platter::workspace::Error) -> Self {
        ReleaseError::Other("workspace error".to_string())
    }
}

impl std::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            ReleaseError::RepositoryUnavailable { url, reason } => {
                write!(f, "Repository unavailable: {}: {}", url, reason)
            }
            ReleaseError::NoUnreleasedChanges => write!(f, "No unreleased changes"),
            ReleaseError::NoVersion => write!(f, "No version"),
            ReleaseError::OddPendingVersion { version } => {
                write!(f, "Odd pending version: {}", version)
            }
            ReleaseError::NoSuchTag => write!(f, "No such tag"),
            ReleaseError::NoDisperseConfig => write!(f, "No disperse config"),
            ReleaseError::PreDistCommandFailed { command, status } => write!(
                f,
                "Pre-dist command failed: {}: {}",
                command,
                status.map_or_else(|| "unknown".to_string(), |s| s.to_string())
            ),
            ReleaseError::UploadCommandFailed {
                command,
                status,
                reason: _,
            } => write!(
                f,
                "Upload command failed: {}: {}",
                command,
                status.map_or_else(|| "unknown".to_string(), |s| s.to_string())
            ),
            ReleaseError::VerifyCommandFailed { command, status } => write!(
                f,
                "Verify command failed: {}: {}",
                command,
                status.map_or_else(|| "unknown".to_string(), |s| s.to_string())
            ),
            ReleaseError::CommitFailed(msg) => write!(f, "Commit failed: {}", msg),
            ReleaseError::RecentCommits {
                min_commit_age,
                commit_age,
            } => write!(
                f,
                "Last commit is {} days old, but minimum is {}",
                commit_age, min_commit_age
            ),
            ReleaseError::ReleaseTagExists {
                project,
                tag,
                version,
            } => write!(
                f,
                "Release tag already exists: {} {} {}",
                project,
                tag,
                version.to_string()
            ),
            ReleaseError::CreateTagFailed {
                tag_name, status, ..
            } => write!(
                f,
                "Create tag failed: {}: {}",
                tag_name,
                status.map_or_else(|| "unknown".to_string(), |s| s.to_string())
            ),
            ReleaseError::Other(msg) => write!(f, "{}", msg),
            ReleaseError::CIFailed(n) => write!(f, "CI failed: {}", n),
            ReleaseError::CIPending(n) => write!(f, "CI pending: {}", n),
            ReleaseError::PublishArtifactsFailed(msg) => {
                write!(f, "Publish artifacts failed: {}", msg)
            }
            ReleaseError::DistCreationFailed => write!(f, "Dist creation failed"),
            ReleaseError::NoPublicBranch => write!(f, "No public branch"),
        }
    }
}

impl std::error::Error for ReleaseError {}

fn is_git_repo(repository: &dyn breezyshim::repository::Repository) -> bool {
    use breezyshim::repository::PyRepository;
    use pyo3::prelude::*;

    // Try to downcast to GenericRepository which implements PyRepository
    if let Some(py_repo) = repository
        .as_any()
        .downcast_ref::<breezyshim::repository::GenericRepository>()
    {
        pyo3::Python::with_gil(|py| py_repo.to_object(py).bind(py).hasattr("_git")).unwrap()
    } else {
        // If it's not a GenericRepository, we can't determine if it's a git repo
        // This might happen with test repositories or other special implementations
        panic!("Cannot determine if repository is git: not a GenericRepository");
    }
}

#[derive(Debug)]
struct RecentCommits {
    min_commit_age: i64,
    commit_age: i64,
}

impl std::fmt::Display for RecentCommits {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(
            f,
            "Last commit is {} days old, but minimum is {}",
            self.commit_age, self.min_commit_age
        )
    }
}

impl std::error::Error for RecentCommits {}

fn check_release_age(
    branch: &dyn breezyshim::branch::Branch,
    cfg: &ProjectConfig,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<(), RecentCommits> {
    let rev = branch
        .repository()
        .get_revision(&branch.last_revision())
        .unwrap();
    if let Some(timeout_days) = cfg.release_timeout {
        let commit_time = rev.datetime();
        let time_delta = now.signed_duration_since(commit_time);
        if (time_delta.num_days() as u64) < timeout_days {
            return Err(RecentCommits {
                min_commit_age: timeout_days as i64,
                commit_age: time_delta.num_days(),
            });
        }
    }

    Ok(())
}

async fn publish_artifacts(
    ws: &silver_platter::workspace::Workspace,
    tag_name: &str,
    dry_run: bool,
    gh: &octocrab::Octocrab,
    cfg: &ProjectConfig,
    pypi_paths: &[&std::path::Path],
    gh_repo: Option<&octocrab::models::Repository>,
) -> Result<Vec<std::path::PathBuf>, ReleaseError> {
    let mut artifacts = vec![];
    // Wait for CI to go green
    if let Some(gh_repo) = gh_repo {
        if dry_run {
            log::info!("In dry-run mode, so unable to wait for CI");
        } else {
            disperse::github::wait_for_gh_actions(gh, gh_repo, Some(tag_name), cfg.ci_timeout)
                .await
                .map_err(|e| ReleaseError::CIFailed(e.to_string()))?;
        }
    }

    if !pypi_paths.is_empty() {
        artifacts.extend(pypi_paths.iter().map(|x| x.to_path_buf()));
        if dry_run {
            log::info!("skipping twine upload due to dry run mode")
        } else if !cfg.twine_upload.unwrap_or(false) {
            log::info!("skipping twine upload; disabled in config")
        } else {
            disperse::python::upload_python_artifacts(ws.local_tree(), pypi_paths).map_err(
                |e| ReleaseError::UploadCommandFailed {
                    command: "twine upload".to_string(),
                    status: None,
                    reason: Some(e.to_string()),
                },
            )?;
        }
    }
    if ws
        .local_tree()
        .has_filename(std::path::Path::new("Cargo.toml"))
    {
        if dry_run {
            log::info!("skipping cargo upload due to dry run mode");
        } else {
            disperse::cargo::publish(ws.local_tree(), std::path::Path::new(".")).map_err(|e| {
                ReleaseError::UploadCommandFailed {
                    command: "cargo publish".to_string(),
                    status: None,
                    reason: Some(e.to_string()),
                }
            })?;
        }
    }
    for loc in cfg.tarball_location.iter() {
        if dry_run {
            log::info!("skipping scp to {} due to dry run mode", loc);
        } else {
            let args = artifacts
                .iter()
                .map(|s| s.to_path_buf().into_os_string())
                .chain([std::ffi::OsString::from(loc)])
                .collect::<Vec<std::ffi::OsString>>();
            match std::process::Command::new("scp")
                .args(args.clone())
                .status()
            {
                Ok(status) => {
                    if !status.success() {
                        return Err(ReleaseError::UploadCommandFailed {
                            command: format!(
                                "scp {}",
                                args.into_iter()
                                    .map(|s| s.into_string().unwrap())
                                    .collect::<Vec<String>>()
                                    .join(" ")
                            ),
                            status: Some(status),
                            reason: None,
                        });
                    }
                }
                Err(e) => {
                    return Err(ReleaseError::UploadCommandFailed {
                        command: format!(
                            "scp {}",
                            args.into_iter()
                                .map(|s| s.into_string().unwrap())
                                .collect::<Vec<String>>()
                                .join(" ")
                        ),
                        status: None,
                        reason: Some(e.to_string()),
                    });
                }
            }
        }
    }
    Ok(artifacts)
}

fn determine_verify_command(cfg: &ProjectConfig, wt: &dyn WorkingTree) -> Option<String> {
    if let Some(verify_command) = cfg.verify_command.as_ref() {
        Some(verify_command.clone())
    } else if wt.has_filename(Path::new("tox.ini")) {
        Some("tox".to_string())
    } else if wt.has_filename(Path::new("Cargo.toml")) {
        Some("cargo test --all".to_string())
    } else {
        None
    }
}

async fn launchpad_client() -> Result<&'static launchpadlib::r#async::client::Client, ReleaseError>
{
    static LAUNCHPAD_CLIENT: tokio::sync::OnceCell<launchpadlib::r#async::client::Client> =
        tokio::sync::OnceCell::const_new();

    LAUNCHPAD_CLIENT
        .get_or_try_init(|| async {
            launchpadlib::r#async::client::Client::authenticated("launchpad.net", "disperse")
                .await
                .map_err(|e| ReleaseError::Other(e.to_string()))
        })
        .await
}

pub async fn release_project(
    repo_url: &str,
    force: Option<bool>,
    new_version: Option<&Version>,
    dry_run: Option<bool>,
    ignore_ci: Option<bool>,
    ignore_verify_command: Option<bool>,
    preserve_temp: bool,
) -> Result<(String, Version), ReleaseError> {
    let force = force.unwrap_or(false);
    let dry_run = dry_run.unwrap_or(false);
    let ignore_ci = ignore_ci.unwrap_or(false);
    let ignore_verify_command = ignore_verify_command.unwrap_or(false);
    let now = chrono::Utc::now();

    let (local_wt, branch) = match breezyshim::controldir::open_tree_or_branch(repo_url, None, None)
    {
        Ok(x) => x,
        Err(e) => {
            return Err(ReleaseError::RepositoryUnavailable {
                url: repo_url.to_string(),
                reason: e.to_string(),
            });
        }
    };

    let mut public_repo_url = None;
    let mut public_branch = None;
    let mut local_branch = None;

    if branch.user_transport().base().scheme() == "file" {
        local_branch = Some(branch);
        if let Some(public_branch_url) = local_branch.as_ref().unwrap().get_public_branch() {
            log::info!("Using public branch {}", &public_branch_url);
            let url: url::Url = public_branch_url.as_str().parse().unwrap();
            let url = disperse::drop_segment_parameters(&url);
            public_repo_url = Some(url.clone());
            public_branch = Some(breezyshim::branch::open(&url).map_err(|e| {
                ReleaseError::RepositoryUnavailable {
                    url: url.to_string(),
                    reason: e.to_string(),
                }
            })?);
        } else if let Some(submit_branch_url) = local_branch.as_ref().unwrap().get_submit_branch() {
            let url: url::Url = submit_branch_url.parse().unwrap();
            let url = disperse::drop_segment_parameters(&url);
            log::info!("Using public branch {}", &submit_branch_url);
            public_repo_url = Some(url.clone());
            public_branch = Some(breezyshim::branch::open(&url).map_err(|e| {
                ReleaseError::RepositoryUnavailable {
                    url: url.to_string(),
                    reason: e.to_string(),
                }
            })?);
        } else if let Some(push_location) = local_branch.as_ref().unwrap().get_push_location() {
            let url: url::Url = push_location.parse().unwrap();
            let url = disperse::drop_segment_parameters(&url);
            log::info!("Using public branch {}", &push_location);
            public_repo_url = Some(url.clone());
            public_branch = Some(breezyshim::branch::open(&url).map_err(|e| {
                ReleaseError::RepositoryUnavailable {
                    url: url.to_string(),
                    reason: e.to_string(),
                }
            })?);
        }
    } else if ["git+ssh", "https", "http", "git"].contains(&branch.user_transport().base().scheme())
    {
        public_repo_url = Some(branch.user_transport().base());
        public_branch = Some(branch);
    } else {
        log::info!(
            "Unknown repository type. Scheme: {}",
            branch.user_transport().base().scheme()
        );
    }

    if let Some(public_repo_url) = &public_repo_url {
        log::info!("Found public repository URL: {}", public_repo_url);
    }

    if let Some(public_branch) = &public_branch {
        log::info!(
            "Found public branch: {}",
            public_branch.user_transport().base()
        );
    }

    if let Some(local_branch) = &local_branch {
        log::info!(
            "Found local branch: {}",
            local_branch.user_transport().base()
        );
    }

    if public_branch.is_none() && local_branch.is_none() {
        return Err(ReleaseError::NoPublicBranch);
    }

    let mut wsbuilder = silver_platter::workspace::Workspace::builder();

    if let Some(public_branch) = public_branch.take() {
        let generic_branch = public_branch
            .as_any()
            .downcast_ref::<breezyshim::branch::GenericBranch>()
            .expect("Expected GenericBranch")
            .clone();
        wsbuilder = wsbuilder.main_branch(generic_branch);
    }

    if let Some(local_branch) = local_branch.take() {
        let generic_branch = local_branch
            .as_any()
            .downcast_ref::<breezyshim::branch::GenericBranch>()
            .expect("Expected GenericBranch")
            .clone();
        wsbuilder = wsbuilder.cached_branch(generic_branch);
    }

    let mut ws = wsbuilder.build().unwrap();

    if preserve_temp {
        ws.defer_destroy();
    }

    let cfg = match disperse::project_config::read_project_with_fallback(ws.local_tree()) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::error!("Unable to read project configuration: {}", e);
            NO_DISPERSE_CONFIG.inc();
            return Err(ReleaseError::NoDisperseConfig);
        }
    };

    let name = if let Some(name) = cfg.name.as_ref() {
        Some(name.clone())
    } else if ws.local_tree().has_filename(Path::new("pyproject.toml")) {
        disperse::python::find_name_in_pyproject_toml(ws.local_tree())
    } else {
        None
    };

    let name = if let Some(name) = name {
        name
    } else {
        public_repo_url
            .as_ref()
            .map(|u| {
                u.as_str()
                    .rsplit('/')
                    .next()
                    .map(|s| s.to_string())
                    .unwrap_or_default()
            })
            .unwrap_or_else(|| "".to_string())
    };

    let mut launchpad_project = if let Some(launchpad) = cfg.launchpad.as_ref() {
        disperse::launchpad::get_project(launchpad_client().await?, &launchpad.project)
            .await
            .ok()
    } else {
        None
    };

    let mut launchpad_series =
        if let Some(series) = cfg.launchpad.as_ref().and_then(|l| l.series.as_ref()) {
            let lp = launchpad_client().await?;
            let series = disperse::launchpad::find_project_series(
                lp,
                &launchpad_project.as_ref().unwrap().self_().unwrap(),
                Some(series),
                None,
            )
            .await
            .map_err(ReleaseError::Other)?;
            let b = series.branch();
            public_repo_url = b.get(lp).await.unwrap().web_link;
            if let Some(url) = &public_repo_url {
                let main_branch_box = breezyshim::branch::open(url).unwrap();
                let main_branch = main_branch_box
                    .as_any()
                    .downcast_ref::<breezyshim::branch::GenericBranch>()
                    .expect("Expected GenericBranch")
                    .clone();
                ws.set_main_branch(main_branch).unwrap();
            }
            // TODO: Check for git repository
            Some(series)
        } else {
            None
        };

    let mut gh_repo = None;

    let gh = disperse::github::login().map_err(|e| ReleaseError::Other(e.to_string()))?;

    if let Some(github) = cfg.github.as_ref() {
        let url = &github.url;
        public_repo_url = Some(url.parse().unwrap());
        let main_branch_box = breezyshim::branch::open(public_repo_url.as_ref().unwrap()).unwrap();
        let main_branch = main_branch_box
            .as_any()
            .downcast_ref::<breezyshim::branch::GenericBranch>()
            .expect("Expected GenericBranch")
            .clone();
        ws.set_main_branch(main_branch).unwrap();
        gh_repo = Some(
            disperse::github::get_github_repo(&gh, public_repo_url.as_ref().unwrap())
                .await
                .map_err(|e| ReleaseError::Other(e.to_string()))?,
        );
        match disperse::github::check_gh_repo_action_status(
            &gh,
            gh_repo.as_ref().unwrap(),
            github.branch.as_deref(),
        )
        .await
        {
            Ok(disperse::github::GitHubCIStatus::Ok) => {
                log::info!("GitHub action succeeded");
            }
            Ok(disperse::github::GitHubCIStatus::Failed { html_url, sha }) => {
                let html_url = html_url.unwrap_or_else(|| "unknown".to_string());
                if ignore_ci {
                    CI_IGNORED_COUNT.with_label_values(&[&name]).inc();
                    log::warn!("Ignoring failing CI: {}", html_url);
                } else {
                    log::error!("CI failed: {}", html_url);
                    log::info!("Pass --ignore-ci to ignore failing CI");
                    return Err(ReleaseError::CIFailed(format!(
                        "for revision {}: {}",
                        sha, html_url
                    )));
                }
            }
            Ok(disperse::github::GitHubCIStatus::Pending { html_url, sha }) => {
                let html_url = html_url.unwrap_or_else(|| "unknown".to_string());
                if ignore_ci {
                    CI_IGNORED_COUNT.with_label_values(&[&name]).inc();
                    log::warn!("Ignoring failing CI: {}", html_url);
                } else {
                    log::error!("CI pending: {}", html_url);
                    log::info!("Pass --ignore-ci to ignore pending CI");
                    return Err(ReleaseError::CIPending(format!(
                        "for revision {}: {}",
                        sha, html_url
                    )));
                }
            }
            Err(e) => {
                log::error!("Unable to check CI status: {}", e);
                return Err(ReleaseError::CIFailed(e.to_string()));
            }
        }
    }

    let public_repo_url = if let Some(public_repo_url) = public_repo_url.as_ref() {
        public_repo_url.clone()
    } else {
        return Err(ReleaseError::NoPublicBranch);
    };

    let mut possible_urls: Vec<(url::Url, Option<String>)> = vec![];
    if ws.local_tree().has_filename(Path::new("setup.cfg")) {
        possible_urls.extend(
            disperse::python::read_project_urls_from_setup_cfg(
                ws.local_tree()
                    .abspath(Path::new("setup.cfg"))
                    .unwrap()
                    .as_path(),
            )
            .map_err(|e| ReleaseError::Other(e.to_string()))?,
        );
    }
    if ws.local_tree().has_filename(Path::new("pyproject.toml")) {
        possible_urls.extend(
            disperse::python::read_project_urls_from_pyproject_toml(
                ws.local_tree()
                    .abspath(Path::new("pyproject.toml"))
                    .unwrap()
                    .as_path(),
            )
            .map_err(|e| ReleaseError::Other(e.to_string()))?,
        );
    }
    possible_urls.push((public_repo_url, ws.main_branch().map(|b| b.name().unwrap())));

    for (parsed_url, branch_name) in possible_urls.iter() {
        match parsed_url.host_str() {
            Some("github.com") => {
                if gh_repo.is_some() {
                    continue;
                }
                gh_repo = Some(
                    disperse::github::get_github_repo(&gh, parsed_url)
                        .await
                        .map_err(|e| ReleaseError::Other(e.to_string()))?,
                );
                match disperse::github::check_gh_repo_action_status(
                    &gh,
                    gh_repo.as_ref().unwrap(),
                    branch_name.as_deref(),
                )
                .await
                {
                    Ok(disperse::github::GitHubCIStatus::Ok) => (),
                    Ok(disperse::github::GitHubCIStatus::Failed { html_url, sha }) => {
                        if ignore_ci {
                            log::warn!("Ignoring failing CI");
                            CI_IGNORED_COUNT.with_label_values(&[&name]).inc();
                        } else {
                            return Err(ReleaseError::CIFailed(format!(
                                "for revision {}: {}",
                                sha,
                                html_url.unwrap_or_else(|| "unknown".to_string())
                            )));
                        }
                    }
                    Ok(disperse::github::GitHubCIStatus::Pending { sha, html_url }) => {
                        if ignore_ci {
                            log::warn!("Ignoring pending CI");
                            CI_IGNORED_COUNT.with_label_values(&[&name]).inc();
                        } else {
                            return Err(ReleaseError::CIPending(format!(
                                "for revision {}: {}",
                                sha,
                                html_url.unwrap_or_else(|| "unknown".to_string())
                            )));
                        }
                    }
                    Err(e) => {
                        log::error!("Unable to check CI status: {}", e);
                        return Err(ReleaseError::CIFailed(e.to_string()));
                    }
                }
                break;
            }
            Some("launchpad.net") => {
                let lp = launchpad_client().await?;
                let parts = parsed_url.path_segments().unwrap().collect::<Vec<_>>();
                launchpad_project = Some(
                    disperse::launchpad::get_project(lp, parts[0])
                        .await
                        .map_err(ReleaseError::Other)?,
                );
                if parts.len() > 1 && !parts[1].starts_with('+') {
                    launchpad_series = Some(
                        disperse::launchpad::find_project_series(
                            lp,
                            &launchpad_project.as_ref().unwrap().self_().unwrap(),
                            Some(parts[1]),
                            None,
                        )
                        .await
                        .map_err(ReleaseError::Other)?,
                    );
                }
            }
            _ => {
                log::debug!("Unknown host: {}", parsed_url);
            }
        }
    }

    if !disperse::check_new_revisions(
        &ws.local_tree().branch(),
        cfg.news_file.as_ref().map(Path::new),
    )
    .map_err(|e| ReleaseError::Other(e.to_string()))?
    {
        NO_UNRELEASED_CHANGES_COUNT
            .with_label_values(&[&name])
            .inc();
        log::info!("No new revisions");
        return Err(ReleaseError::NoUnreleasedChanges);
    }

    if let Err(RecentCommits {
        min_commit_age,
        commit_age,
    }) = check_release_age(&ws.local_tree().branch(), &cfg, now)
    {
        RECENT_COMMITS_COUNT.with_label_values(&[&name]).inc();
        if !force {
            return Err(ReleaseError::RecentCommits {
                min_commit_age,
                commit_age,
            });
        }
    }

    let new_version: Version = new_version.map_or_else(
        || {
            let new_version =
                pick_new_version(ws.local_tree(), &cfg).map_err(ReleaseError::Other)?;
            log::info!("Picked new version: {}", new_version.to_string());
            Ok::<Version, ReleaseError>(new_version)
        },
        |v| Ok(v.clone()),
    )?;

    if let Some(pre_dist_command) = cfg.pre_dist_command.as_ref() {
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(pre_dist_command)
            .current_dir(ws.local_tree().abspath(Path::new(".")).unwrap())
            .status()
        {
            Ok(s) => {
                if !s.success() {
                    PRE_DIST_COMMAND_FAILED.with_label_values(&[&name]).inc();
                    return Err(ReleaseError::PreDistCommandFailed {
                        command: pre_dist_command.clone(),
                        status: Some(s),
                    });
                }
            }
            Err(_e) => {
                PRE_DIST_COMMAND_FAILED.with_label_values(&[&name]).inc();
                return Err(ReleaseError::PreDistCommandFailed {
                    command: pre_dist_command.clone(),
                    status: None,
                });
            }
        }
    }

    let verify_command = determine_verify_command(&cfg, ws.local_tree());

    log::info!("releasing {}", new_version.to_string());
    let (news_file, release_changes) = if let Some(news_file_path) = cfg.news_file.as_ref() {
        let news_file =
            disperse::news_file::NewsFile::new(ws.local_tree(), Path::new(news_file_path))
                .map_err(|e| ReleaseError::Other(e.to_string()))?;
        let release_changes = news_file
            .mark_released(&new_version, &now.date_naive())
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
        (Some(news_file), Some(release_changes))
    } else {
        (None, None)
    };

    for update_version in cfg.update_version.as_ref().unwrap_or(&vec![]) {
        disperse::custom::update_version_in_file(
            ws.local_tree(),
            &update_version.path,
            &update_version.new_line,
            update_version.r#match.as_deref(),
            &new_version,
            disperse::Status::Final,
        )
        .map_err(ReleaseError::Other)?;
    }

    for update_manpage in cfg.update_manpages.as_ref().unwrap_or(&vec![]) {
        for path in disperse::iter_glob(ws.local_tree(), update_manpage.to_str().unwrap()) {
            disperse::manpage::update_version_in_manpage(
                ws.local_tree(),
                &path,
                &new_version,
                now.date_naive(),
            )
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
        }
    }

    if ws.local_tree().has_filename(Path::new("Cargo.toml")) {
        disperse::cargo::update_version(ws.local_tree(), new_version.to_string().as_str())
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
    }
    if ws.local_tree().has_filename(Path::new("pyproject.toml")) {
        disperse::python::update_version_in_pyproject_toml(ws.local_tree(), &new_version)
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
    }
    let revid = ws
        .local_tree()
        .build_commit()
        .message(format!("Release {}.", new_version.to_string()).as_str())
        .commit()
        .map_err(|e| ReleaseError::CommitFailed(e.to_string()))?;

    if let Some(verify_command) = verify_command {
        match std::process::Command::new("sh")
            .arg("-c")
            .arg(&verify_command)
            .current_dir(ws.local_tree().abspath(Path::new(".")).unwrap())
            .status()
        {
            Ok(s) => {
                if !s.success() {
                    VERIFY_COMMAND_FAILED.with_label_values(&[&name]).inc();
                    if !ignore_verify_command {
                        return Err(ReleaseError::VerifyCommandFailed {
                            command: verify_command.clone(),
                            status: Some(s),
                        });
                    }
                }
            }
            Err(_e) => {
                VERIFY_COMMAND_FAILED.with_label_values(&[&name]).inc();
                if !ignore_verify_command {
                    return Err(ReleaseError::VerifyCommandFailed {
                        command: verify_command.clone(),
                        status: None,
                    });
                }
            }
        }
    }

    let tag_name = disperse::version::expand_tag(cfg.tag_name.as_ref().unwrap(), &new_version);
    let tags = ws.local_tree().branch().tags().unwrap();
    if tags.has_tag(tag_name.as_str()) {
        RELEASE_TAG_EXISTS.with_label_values(&[&name]).inc();
        // Maybe there's a pending pull request merging new_version?
        // TODO(jelmer): Do some more verification. Expect: release tag
        // has one additional revision that's not on our branch.
        return Err(ReleaseError::ReleaseTagExists {
            project: name,
            version: new_version,
            tag: tag_name,
        });
    }
    log::info!("Creating tag {}", tag_name);
    if is_git_repo(&ws.local_tree().branch().repository()) {
        match std::process::Command::new("git")
            .arg("tag")
            .arg("-as")
            .arg(&tag_name)
            .arg("-m")
            .arg(format!("Release {}", new_version.to_string()))
            .current_dir(ws.local_tree().abspath(Path::new(".")).unwrap())
            .status()
        {
            Ok(s) => {
                if !s.success() {
                    return Err(ReleaseError::CreateTagFailed {
                        tag_name: tag_name.clone(),
                        status: Some(s),
                        reason: Some("git tag failed".to_string()),
                    });
                }
            }
            Err(e) => {
                return Err(ReleaseError::CreateTagFailed {
                    tag_name: tag_name.clone(),
                    status: None,
                    reason: Some(e.to_string()),
                });
            }
        }
    } else {
        tags.set_tag(tag_name.as_str(), &ws.local_tree().last_revision().unwrap())
            .map_err(|e| ReleaseError::CreateTagFailed {
                tag_name: tag_name.clone(),
                status: None,
                reason: Some(e.to_string()),
            })?;
    }

    log::info!("Creating Python artifacts");
    let pypi_paths = if ws.local_tree().has_filename(Path::new("setup.py")) {
        disperse::python::create_setup_py_artifacts(ws.local_tree()).unwrap()
    } else if ws.local_tree().has_filename(Path::new("pyproject.toml")) {
        disperse::python::create_python_artifacts(ws.local_tree()).unwrap()
    } else {
        vec![]
    };

    if !dry_run {
        log::info!(
            "Pushing tag {} to {}",
            tag_name,
            ws.main_branch().unwrap().get_user_url()
        );
        ws.push_tags(hashmap! {
            tag_name.clone() => revid.clone(),
        })
        .map_err(|e| ReleaseError::CreateTagFailed {
            tag_name: tag_name.clone(),
            status: None,
            reason: Some(e.to_string()),
        })?;
    }

    let result = publish_artifacts(
        &ws,
        &tag_name,
        dry_run,
        &gh,
        &cfg,
        pypi_paths
            .iter()
            .map(|p| p.as_path())
            .collect::<Vec<_>>()
            .as_slice(),
        gh_repo.as_ref(),
    )
    .await;

    let artifacts = match result {
        Ok(artifacts) => artifacts,
        Err(e) => {
            log::error!("Failed to publish artifacts: {}", e);
            log::info!("Deleting remote tag {}", tag_name);
            if !dry_run {
                tags.delete_tag(tag_name.as_str())
                    .map_err(|e| ReleaseError::Other(e.to_string()))?;
            }
            return Err(ReleaseError::PublishArtifactsFailed(e.to_string()));
        }
    };

    // At this point, it's official - so let's push.
    if !dry_run {
        match ws.push(None) {
            Ok(_) => {}
            Err(silver_platter::workspace::Error::BrzError(
                BrzError::ProtectedBranchHookDeclined(..),
            )) => {
                BRANCH_PROTECTED_COUNT.with_label_values(&[&name]).inc();
                log::info!(
                    "{} is protected; proposing merge instead",
                    ws.local_tree()
                        .branch()
                        .name()
                        .unwrap_or_else(|| "branch".to_string())
                );
                let commit_message = format!("Merge release of {}", new_version.to_string());
                let mp = if !dry_run {
                    let (mp, _is_new) = ws.propose(
                        format!("release-{}", new_version.to_string()).as_str(),
                        format!("Merge release of {}", new_version.to_string()).as_str(),
                        None,
                        None,
                        None,
                        Some(hashmap! { tag_name.clone() => revid }),
                        Some(vec!["release".to_string()]),
                        None,
                        Some(commit_message.as_str()),
                        None,
                        None,
                        None,
                        None,
                        None,
                        None,
                        None, // work_in_progress
                    )?;
                    Some(mp)
                } else {
                    None
                };

                if let Some(mp) = mp {
                    log::info!("Created merge proposal: {}", mp.url().unwrap());

                    if mp.supports_auto_merge() {
                        mp.merge(true)
                            .map_err(|e| ReleaseError::Other(e.to_string()))?;
                    }
                }
            }
            Err(e) => {
                log::info!("Failed to push: {}", e);
                return Err(e.into());
            }
        }
    }

    if let Some(gh_repo) = gh_repo.as_ref() {
        if dry_run {
            log::info!("skipping creation of github release due to dry run mode");
        } else {
            disperse::github::create_github_release(
                &gh,
                gh_repo,
                tag_name.as_str(),
                &new_version.to_string(),
                release_changes.as_deref(),
            )
            .await
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
        }
    }

    if let Some(launchpad_project) = launchpad_project.as_ref() {
        if dry_run {
            log::info!("skipping upload of tarball to Launchpad");
        } else {
            let lp = launchpad_client().await?;
            let lp_release = disperse::launchpad::ensure_release(
                lp,
                &launchpad_project.self_().unwrap(),
                &new_version.to_string(),
                launchpad_series.as_ref().map(|s| s.name.as_str()),
                release_changes.as_deref(),
            )
            .await
            .map_err(ReleaseError::Other)?;
            disperse::launchpad::add_release_files(lp, &lp_release, artifacts)
                .await
                .map_err(ReleaseError::Other)?;
        }
    }

    // TODO(jelmer): Mark any news bugs in NEWS as fixed [later]
    // * Commit:
    //  * Update NEWS and version strings for next version
    let mut new_pending_version: Version = new_version.clone();
    disperse::version::increase_version(&mut new_pending_version, -1);
    assert!(new_pending_version > new_version);
    log::info!("Using new version {}", new_pending_version.to_string());
    if let Some(news_file) = news_file {
        news_file
            .add_pending(&new_pending_version)
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
        ws.local_tree()
            .build_commit()
            .message(format!("Start on {}", new_pending_version.to_string()).as_str())
            .commit()
            .map_err(|e| ReleaseError::Other(e.to_string()))?;
        if !dry_run {
            ws.push(None)
                .map_err(|e| ReleaseError::Other(e.to_string()))?;
        }
    }
    if let Some(launchpad_project) = launchpad_project.as_ref() {
        if dry_run {
            log::info!(
                "Skipping creation of new mileston {} on Launchpad",
                new_pending_version.to_string(),
            );
        } else {
            disperse::launchpad::create_milestone(
                launchpad_client().await?,
                &launchpad_project.self_().unwrap(),
                &new_pending_version.to_string(),
                launchpad_series.as_ref().map(|s| s.name.as_str()),
            )
            .await
            .map_err(ReleaseError::Other)?;
        }
    }
    if !dry_run {
        if let Some(public_branch) = ws.main_branch() {
            if let Some(local_wt) = local_wt.as_ref() {
                local_wt.pull(public_branch, None, None, None).unwrap();
            } else if let Some(local_branch) = local_branch.as_ref() {
                local_branch.pull(public_branch, None).unwrap();
            }
        } else {
            log::info!("No public branch to pull from");
        }
    }

    RELEASED_COUNT.with_label_values(&[&name]).inc();
    Ok((name, new_version))
}

async fn release_many(
    urls: &[String],
    new_version: Option<String>,
    ignore_ci: Option<bool>,
    ignore_verify_command: Option<bool>,
    dry_run: Option<bool>,
    discover: bool,
    force: Option<bool>,
    preserve_temp: bool,
) -> i32 {
    let mut failed: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut success: Vec<String> = Vec::new();
    let mut ret = 0;
    for url in urls {
        if url != "." {
            log::info!("Processing {}", url);
        }
        match release_project(
            url,
            force,
            new_version
                .as_ref()
                .map(|v| v.as_str().parse().unwrap())
                .as_ref(),
            dry_run,
            ignore_ci,
            ignore_verify_command,
            preserve_temp,
        )
        .await
        {
            Err(ReleaseError::RecentCommits {
                min_commit_age,
                commit_age,
            }) => {
                log::info!("Recent commits exist ({} < {})", min_commit_age, commit_age);
                skipped.push((
                    url.to_string(),
                    format!("Recent commits exist ({} < {})", min_commit_age, commit_age),
                ));
                if !discover {
                    ret = 1;
                }
            }
            Err(ReleaseError::VerifyCommandFailed { command, .. }) => {
                log::error!("Verify command ({}) failed to run.", command);
                failed.push((
                    url.to_string(),
                    format!("Verify command ({}) failed to run.", command),
                ));
                ret = 1;
            }
            Err(ReleaseError::PreDistCommandFailed { command, .. }) => {
                log::error!("Pre-Dist command ({}) failed to run.", command);
                failed.push((
                    url.to_string(),
                    format!("Pre-Dist command ({}) failed to run.", command),
                ));
                ret = 1;
            }
            Err(ReleaseError::UploadCommandFailed { command, .. }) => {
                log::error!("Upload command ({}) failed to run.", command);
                failed.push((
                    url.to_string(),
                    format!("Upload command ({}) failed to run.", command),
                ));
                ret = 1;
            }
            Err(ReleaseError::ReleaseTagExists {
                project,
                tag,
                version,
            }) => {
                log::warn!(
                    "{}: Release tag {} for version {} exists. Unmerged release commit?",
                    project,
                    tag,
                    version.to_string(),
                );
                skipped.push((
                    url.to_string(),
                    format!(
                        "Release tag {} for version {} exists",
                        tag,
                        version.to_string()
                    ),
                ));
                if !discover {
                    ret = 1;
                }
            }
            Err(ReleaseError::DistCreationFailed) => {
                log::error!("Dist creation failed to run.");
                failed.push((url.to_string(), "Dist creation failed to run.".to_string()));
                ret = 1;
            }
            Err(ReleaseError::NoUnreleasedChanges) => {
                log::error!("No unreleased changes");
                skipped.push((url.to_string(), "No unreleased changes".to_string()));
                if !discover {
                    ret = 1;
                }
            }
            Err(ReleaseError::NoDisperseConfig) => {
                log::error!("No configuration for disperse");
                skipped.push((url.to_string(), "No configuration for disperse".to_string()));
                if !discover {
                    ret = 1;
                }
            }
            Err(ReleaseError::CIPending(n)) => {
                log::error!("CI checks not finished yet: {}", n);
                failed.push((
                    url.to_string(),
                    format!("CI checks not finished yet: {}", n),
                ));
                ret = 1;
            }
            Err(ReleaseError::CIFailed(n)) => {
                log::error!("GitHub check failed: {}", n);
                failed.push((url.to_string(), format!("GitHub check failed: {}", n)));
                ret = 1;
            }
            Err(ReleaseError::RepositoryUnavailable { url, reason }) => {
                log::error!("Repository is unavailable: {}: {}", url, reason);
                failed.push((
                    url.to_string(),
                    format!("Repository is unavailable: {}: {}", url, reason),
                ));
                ret = 1;
            }
            Err(ReleaseError::OddPendingVersion { version }) => {
                log::error!("Odd pending version: {}", version);
                failed.push((url.to_string(), format!("Odd pending version: {}", version)));
                ret = 1;
            }
            Err(ReleaseError::NoVersion) => {
                log::error!("No version");
                failed.push((url.to_string(), "No version".to_string()));
                ret = 1;
            }
            Err(ReleaseError::NoSuchTag) => {
                log::error!("No such tag");
                failed.push((url.to_string(), "No such tag".to_string()));
                ret = 1;
            }
            Err(ReleaseError::CreateTagFailed { .. }) => {
                log::error!("Failed to create tag");
                failed.push((url.to_string(), "Failed to create tag".to_string()));
                ret = 1;
            }
            Err(ReleaseError::Other(o)) => {
                log::error!("Other error: {:?}", o);
                failed.push((url.to_string(), format!("Other error: {}", o)));
                ret = 1;
            }
            Err(ReleaseError::CommitFailed(..)) => {
                log::error!("Failed to commit");
                failed.push((url.to_string(), "Failed to commit".to_string()));
                ret = 1;
            }
            Err(ReleaseError::PublishArtifactsFailed(o)) => {
                log::error!("Failed to publish artifacts: {}", o);
                failed.push((
                    url.to_string(),
                    format!("Failed to publish artifacts: {}", o),
                ));
                ret = 1;
            }
            Err(ReleaseError::NoPublicBranch) => {
                log::error!("No public branch");
                failed.push((url.to_string(), "No public branch".to_string()));
                ret = 1;
            }
            Ok((name, version)) => {
                log::info!("Released {} version {}", name, version.to_string());
                success.push(url.to_string());
            }
        }
    }

    if discover {
        log::info!(
            "{} successfully released, {} skipped, {} failed",
            success.len(),
            skipped.len(),
            failed.len()
        );
    }

    ret
}

fn validate_config(path: &std::path::Path) -> i32 {
    let wt = match workingtree::open(path) {
        Ok(x) => x,
        Err(e) => {
            log::error!("Unable to open working tree: {}", e);
            return 1;
        }
    };

    let cfg = match read_project_with_fallback(&wt) {
        Ok(x) => x,
        Err(e) => {
            log::error!("Unable to read config: {}", e);
            return 1;
        }
    };

    if let Some(news_file) = &cfg.news_file {
        let news_file = wt.basedir().join(news_file);
        if !news_file.exists() {
            log::error!("News file {} does not exist", news_file.display());
            return 1;
        }
    }

    for update_version in cfg.update_version.unwrap_or_default().iter() {
        match disperse::custom::validate_update_version(&wt, update_version) {
            Ok(_) => {}
            Err(e) => {
                log::error!("Invalid update_version: {}", e);
                return 1;
            }
        }
    }

    for update_manpage in cfg.update_manpages.unwrap_or_default().iter() {
        for path in disperse::iter_glob(&wt, update_manpage.to_str().unwrap()) {
            match disperse::manpage::validate_update_manpage(&wt, path.as_path()) {
                Ok(_) => {}
                Err(e) => {
                    log::error!("Invalid update_manpage: {}", e);
                    return 1;
                }
            }
        }
    }

    0
}

fn verify(wt: &dyn WorkingTree) -> Result<(), i32> {
    let cfg = match disperse::project_config::read_project_with_fallback(wt) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::info!("Error loading configuration: {}", e);
            return Err(1);
        }
    };

    let verify_command = determine_verify_command(&cfg, wt);

    if verify_command.is_none() {
        log::info!("No verify command configured or detected");
        return Ok(());
    }

    let verify_command = verify_command.unwrap();

    log::info!("Running verify command: {}", verify_command);

    let mut cmd = std::process::Command::new("sh");
    cmd.arg("-c").arg(verify_command);
    cmd.current_dir(wt.abspath(std::path::Path::new(".")).unwrap());
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::inherit());
    cmd.stderr(std::process::Stdio::inherit());
    let status = cmd.status().unwrap();

    if !status.success() {
        log::error!("Verify command failed");
        return Err(1);
    }

    Ok(())
}

fn init(wt: &dyn WorkingTree) -> Result<(), i32> {
    if wt.has_filename(Path::new("disperse.toml")) {
        log::info!("Already initialized");
        return Ok(());
    }

    let cfg = disperse::project_config::ProjectConfig::default();

    wt.put_file_bytes_non_atomic(
        Path::new("disperse.toml"),
        toml::to_string_pretty(&cfg).unwrap().as_bytes(),
    )
    .map_err(|e| {
        log::error!("Unable to write disperse.toml: {}", e);
        1
    })?;

    wt.add(&[Path::new("disperse.toml")]).map_err(|e| {
        log::error!("Unable to add disperse.toml: {}", e);
        1
    })?;

    wt.build_commit()
        .message("Initialize disperse.toml")
        .commit()
        .map_err(|e| {
            log::error!("Unable to commit initialization: {}", e);
            1
        })?;

    Ok(())
}

fn migrate(wt: &dyn WorkingTree) -> Result<(), i32> {
    if wt.has_filename(Path::new("disperse.toml")) {
        log::info!("Already migrated");
        return Ok(());
    }

    // TODO: Check that the old configuration files have no changes

    let cfg = match disperse::project_config::read_project_with_fallback(wt) {
        Ok(cfg) => cfg,
        Err(e) => {
            log::error!("Unable to read project configuration: {}", e);
            return Err(1);
        }
    };

    wt.put_file_bytes_non_atomic(
        Path::new("disperse.toml"),
        toml::to_string_pretty(&cfg).unwrap().as_bytes(),
    )
    .map_err(|e| {
        log::error!("Unable to write disperse.toml: {}", e);
        1
    })?;

    wt.add(&[Path::new("disperse.toml")]).map_err(|e| {
        log::error!("Unable to add disperse.toml: {}", e);
        1
    })?;

    let mut paths = vec![Path::new("disperse.toml")];

    match wt.remove(&[Path::new("disperse.conf")]) {
        Ok(_) => {
            let p = Path::new("disperse.conf");
            if wt.has_filename(p) {
                paths.push(p);
            }
        }
        Err(BrzError::NoSuchFile(_)) => {}
        Err(e) => {
            log::error!("Unable to remove disperse.conf: {}", e);
        }
    }
    match wt.remove(&[Path::new("release.conf")]) {
        Ok(_) => {
            let p = Path::new("release.conf");
            if wt.has_filename(p) {
                paths.push(p);
            }
        }
        Err(BrzError::NoSuchFile(_)) => {}
        Err(e) => {
            log::error!("Unable to remove release.conf: {}", e);
        }
    }

    wt.build_commit()
        .message("Migrate to disperse.toml")
        .specific_files(paths.as_slice())
        .commit()
        .map_err(|e| {
            log::error!("Unable to commit migration: {}", e);
            1
        })?;

    Ok(())
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    env_logger::builder()
        .format(|buf, record| writeln!(buf, "{}", record.args()))
        .filter(
            None,
            if args.debug {
                log::LevelFilter::Debug
            } else {
                log::LevelFilter::Info
            },
        )
        .init();

    let config = disperse::config::load_config().unwrap().unwrap_or_default();

    log::debug!("Config: {:?}", config);

    pyo3::prepare_freethreaded_python();

    breezyshim::init();
    breezyshim::plugin::load_plugins();

    std::process::exit(match &args.command {
        Commands::Release(release_args) => {
            release_many(
                release_args.url.as_slice(),
                release_args.new_version.clone(),
                Some(release_args.ignore_ci),
                Some(release_args.ignore_verify_command),
                Some(args.dry_run),
                release_args.discover,
                Some(true),
                release_args.preserve_temp,
            )
            .await
        }
        Commands::Discover(discover_args) => {
            let pypi_usernames = match discover_args.pypi_user.as_slice() {
                [] => config
                    .pypi
                    .map(|pypi| vec![pypi.username])
                    .unwrap_or_default(),
                pypi_usernames => pypi_usernames.to_vec(),
            };

            let crates_io_user = match discover_args.crates_io_user.as_ref() {
                Option::None => config.crates_io.map(|crates_io| crates_io.username),
                Some(crates_io_user) => Some(crates_io_user.clone()),
            };

            let pypi_urls = pypi_usernames
                .iter()
                .flat_map(|pypi_username| disperse::python::pypi_discover_urls(pypi_username))
                .flatten()
                .collect::<Vec<_>>();

            let crates_io_urls = match crates_io_user {
                Option::None => {
                    vec![]
                }
                Some(crates_io_user) => {
                    disperse::cargo::get_owned_crates(crates_io_user.as_str()).unwrap()
                }
            };

            let repositories_urls = config
                .repositories
                .and_then(|repositories| repositories.owned)
                .unwrap_or_default();

            let urls: Vec<Url> = vec![pypi_urls, crates_io_urls, repositories_urls]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

            if urls.is_empty() {
                log::error!("No projects found. Specify pypi or crates.io username, or add repositories to config");
                0
            } else {
                let ret = if discover_args.info {
                    info_many(urls.as_slice())
                } else if discover_args.urls {
                    println!(
                        "{}",
                        urls.iter()
                            .map(|u| u.to_string())
                            .collect::<Vec<_>>()
                            .join("\n")
                    );
                    0
                } else {
                    release_many(
                        urls.iter()
                            .map(|x| x.to_string())
                            .collect::<Vec<_>>()
                            .as_slice(),
                        None,
                        Some(false),
                        Some(false),
                        Some(false),
                        true,
                        Some(false),
                        false,
                    )
                    .await
                };
                if let Some(prometheus) = args.prometheus {
                    push_to_gateway(prometheus.as_str()).await.unwrap();
                }
                if discover_args.r#try {
                    0
                } else {
                    ret
                }
            }
        }
        Commands::Validate(args) => validate_config(&args.path),
        Commands::Info(args) => {
            let wt = workingtree::open(args.path.as_ref()).unwrap();
            info(&wt, &wt.branch())
        }
        Commands::Verify(args) => {
            let wt = workingtree::open(args.path.as_ref()).unwrap();
            match verify(&wt) {
                Ok(_) => 0,
                Err(e) => e,
            }
        }
        Commands::Migrate(args) => {
            let wt = workingtree::open(args.path.as_ref()).unwrap();
            match migrate(&wt) {
                Ok(_) => 0,
                Err(e) => e,
            }
        }
        Commands::Init(args) => {
            let wt = workingtree::open(args.path.as_ref()).unwrap();
            match init(&wt) {
                Ok(_) => 0,
                Err(e) => e,
            }
        }
    });
}
