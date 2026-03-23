use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{schemars, tool, tool_handler, tool_router, ServerHandler, ServiceExt};

use breezyshim::branch::Branch;
use breezyshim::repository::Repository;
use breezyshim::tree::Tree;
use breezyshim::workingtree::{self, WorkingTree};
use std::path::{Path, PathBuf};

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ProjectPathParams {
    #[schemars(description = "Path to the project directory (defaults to current directory)")]
    pub path: Option<String>,
}

fn resolve_path(path: Option<&str>) -> PathBuf {
    match path {
        Some(p) => PathBuf::from(p),
        None => PathBuf::from("."),
    }
}

fn get_project_info(path: &Path) -> Result<String, String> {
    let wt = workingtree::open(path)
        .map_err(|e| format!("Unable to open working tree at {}: {}", path.display(), e))?;
    let lock = wt.lock_read();

    let cfg = disperse::project_config::read_project_with_fallback(&wt)
        .map_err(|e| format!("Error loading configuration: {}", e))?;

    let mut output = String::new();

    let name = if let Some(name) = cfg.name.as_ref() {
        Some(name.clone())
    } else if wt.has_filename(Path::new("pyproject.toml")) {
        disperse::python::find_name_in_pyproject_toml(&wt)
    } else {
        None
    };

    if let Some(name) = name {
        output.push_str(&format!("Project: {}\n", name));
    }

    let (mut last_version, last_version_status) = match crate::find_last_version(&wt, &cfg) {
        Ok((Some(v), s)) => (v, s),
        Ok((None, _)) => return Err("No version found".to_string()),
        Err(e) => return Err(format!("Error loading last version: {}", e)),
    };

    output.push_str(&format!("Last release: {}\n", last_version));
    if let Some(status) = last_version_status {
        output.push_str(&format!("  status: {}\n", status));
    }

    let branch = wt.branch();
    let tags = branch
        .tags()
        .map_err(|e| format!("Error getting tags: {}", e))?;

    if let Some(tag_name_template) = cfg.tag_name.as_deref() {
        let tag_name = disperse::version::expand_tag(tag_name_template, &last_version);
        match tags.lookup_tag(&tag_name) {
            Ok(release_revid) => {
                output.push_str(&format!("  tag name: {} ({})\n", tag_name, release_revid));

                let rev = branch
                    .repository()
                    .get_revision(&release_revid)
                    .map_err(|e| format!("Error getting revision: {}", e))?;
                output.push_str(&format!(
                    "  date: {}\n",
                    rev.datetime().format("%Y-%m-%d %H:%M:%S")
                ));

                if rev.revision_id != branch.last_revision() {
                    let graph = branch.repository().get_graph();
                    match graph
                        .iter_lefthand_ancestry(&branch.last_revision(), Some(&[release_revid]))
                    {
                        Ok(iter) => {
                            let missing: Vec<breezyshim::revisionid::RevisionId> = iter
                                .collect::<Result<Vec<_>, _>>()
                                .map_err(|e| format!("Error iterating ancestry: {}", e))?;
                            if missing.last().map(|r| r.is_null()).unwrap_or(false) {
                                output.push_str("  last release not found in ancestry\n");
                            } else if let Some(last) = missing.last() {
                                let first = branch
                                    .repository()
                                    .get_revision(last)
                                    .map_err(|e| format!("Error getting revision: {}", e))?;
                                let first_timestamp = first.datetime();
                                let first_age = chrono::Utc::now()
                                    .signed_duration_since(first_timestamp)
                                    .num_days();
                                output.push_str(&format!(
                                    "  {} revisions since last release. First is {} days old.\n",
                                    missing.len(),
                                    first_age,
                                ));
                            }
                        }
                        Err(e) => {
                            output.push_str(&format!("  error getting ancestry: {}\n", e));
                        }
                    }
                } else {
                    output.push_str("  no revisions since last release\n");
                }
            }
            Err(breezyshim::error::Error::NoSuchTag(name)) => {
                output.push_str(&format!("  tag {} for previous release not found\n", name));
            }
            Err(e) => {
                output.push_str(&format!("  error loading tag: {}\n", e));
            }
        }
    }

    match disperse::find_pending_version(&wt, &cfg) {
        Ok(new_version) => {
            output.push_str(&format!("Pending version: {}\n", new_version));
        }
        Err(disperse::FindPendingVersionError::OddPendingVersion(e)) => {
            output.push_str(&format!("Pending version: {} (odd)\n", e));
        }
        Err(disperse::FindPendingVersionError::NotFound) => {
            disperse::version::increase_version(&mut last_version, -1);
            output.push_str(&format!(
                "No pending version found; would use {}\n",
                last_version
            ));
        }
        Err(disperse::FindPendingVersionError::NoUnreleasedChanges) => {
            output.push_str("No unreleased changes\n");
        }
        Err(disperse::FindPendingVersionError::Other(e)) => {
            output.push_str(&format!("Error finding pending version: {}\n", e));
        }
    }

    std::mem::drop(lock);
    Ok(output)
}

fn validate_project(path: &Path) -> Result<String, String> {
    let wt = workingtree::open(path)
        .map_err(|e| format!("Unable to open working tree at {}: {}", path.display(), e))?;

    let cfg = disperse::project_config::read_project_with_fallback(&wt)
        .map_err(|e| format!("Unable to read config: {}", e))?;

    let mut issues = Vec::new();

    if let Some(news_file) = &cfg.news_file {
        let news_file_path = wt.basedir().join(news_file);
        if !news_file_path.exists() {
            issues.push(format!(
                "News file {} does not exist",
                news_file_path.display()
            ));
        }
    }

    for update_version in cfg.update_version.unwrap_or_default().iter() {
        if let Err(e) = disperse::custom::validate_update_version(&wt, update_version) {
            issues.push(format!("Invalid update_version: {}", e));
        }
    }

    if issues.is_empty() {
        Ok("Configuration is valid.".to_string())
    } else {
        Err(issues.join("\n"))
    }
}

#[derive(Debug, Clone)]
pub struct DisperseServer {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl DisperseServer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(
        description = "Show information about a project: current version, pending version, release status"
    )]
    fn info(&self, Parameters(params): Parameters<ProjectPathParams>) -> Result<String, String> {
        let path = resolve_path(params.path.as_deref());
        get_project_info(&path)
    }

    #[tool(description = "Validate the disperse configuration for a project")]
    fn validate(
        &self,
        Parameters(params): Parameters<ProjectPathParams>,
    ) -> Result<String, String> {
        let path = resolve_path(params.path.as_deref());
        validate_project(&path)
    }
}

#[tool_handler]
impl ServerHandler for DisperseServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Disperse: automation for creation of releases of free software packages. Use the tools to inspect project status, validate configuration, and create releases.".to_string())
    }
}

pub async fn run_mcp_server() -> Result<(), Box<dyn std::error::Error>> {
    let service = DisperseServer::new()
        .serve(rmcp::transport::stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}
