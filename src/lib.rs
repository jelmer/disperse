pub mod cargo;
pub mod config;
pub mod custom;
pub mod github;
pub mod manpage;
pub mod news_file;
pub mod project_config;
pub mod python;
pub mod version;
use breezyshim::branch::Branch;
use breezyshim::tree::Tree;
use log::warn;
use std::path::Path;

pub use version::Version;

pub const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Final,
    Dev,
}

#[cfg(feature = "pyo3")]
impl pyo3::FromPyObject<'_> for Status {
    fn extract(ob: &pyo3::PyAny) -> pyo3::PyResult<Self> {
        let s = ob.extract::<String>()?;
        s.parse()
            .map_err(pyo3::PyErr::new::<pyo3::exceptions::PyValueError, _>)
    }
}

#[cfg(feature = "pyo3")]
impl pyo3::ToPyObject for Status {
    fn to_object(&self, py: pyo3::Python) -> pyo3::PyObject {
        self.to_string().to_object(py)
    }
}

#[cfg(feature = "pyo3")]
impl pyo3::IntoPy<pyo3::PyObject> for Status {
    fn into_py(self, py: pyo3::Python) -> pyo3::PyObject {
        self.to_string().into_py(py)
    }
}

impl ToString for Status {
    fn to_string(&self) -> String {
        match self {
            Status::Final => "final".to_string(),
            Status::Dev => "dev".to_string(),
        }
    }
}

impl std::str::FromStr for Status {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "final" => Ok(Status::Final),
            "dev" => Ok(Status::Dev),
            _ => Err(format!("invalid status: {}", s)),
        }
    }
}

pub fn check_new_revisions(
    branch: &dyn Branch,
    news_file_path: Option<&std::path::Path>,
) -> std::result::Result<bool, Box<dyn std::error::Error>> {
    let tags = branch.tags().unwrap().get_reverse_tag_dict()?;
    let lock = branch.lock_read();
    let repository = branch.repository();
    let graph = repository.get_graph();
    let from_tree = graph
        .iter_lefthand_ancestry(&branch.last_revision(), None)
        .find_map(|revid| {
            let revid = revid.ok()?;
            if tags.contains_key(&revid) {
                Some(repository.revision_tree(&revid))
            } else {
                None
            }
        })
        .unwrap_or(repository.revision_tree(&breezyshim::revisionid::RevisionId::null()))?;

    let last_tree = branch.basis_tree()?;
    let mut delta = breezyshim::intertree::get(&from_tree, &last_tree).compare();
    if let Some(news_file_path) = news_file_path {
        for (i, m) in delta.modified.iter().enumerate() {
            if (m.path.0.as_deref(), m.path.1.as_deref())
                == (Some(news_file_path), Some(news_file_path))
            {
                delta.modified.remove(i);
                break;
            }
        }
    }
    std::mem::drop(lock);
    Ok(delta.has_changed())
}

pub fn find_last_version_in_tags(
    branch: &dyn breezyshim::branch::Branch,
    tag_name: &str,
) -> (Option<Version>, Option<Status>) {
    let rev_tag_dict = branch.tags().unwrap().get_reverse_tag_dict().unwrap();
    let graph = branch.repository().get_graph();

    let (revid, tags) = graph
        .iter_lefthand_ancestry(&branch.last_revision(), None)
        .find_map(|r| {
            let revid = r.ok()?;
            rev_tag_dict.get(&revid).map(|tags| (revid, tags))
        })
        .unwrap();

    for tag in tags {
        let release = match crate::version::unexpand_tag(tag_name, tag) {
            Ok(release) => release,
            Err(_) => continue,
        };
        let status = if revid == branch.last_revision() {
            Status::Final
        } else {
            Status::Dev
        };
        return (Some(release), Some(status));
    }

    warn!("Unable to find any tags matching {}", tag_name);
    (None, None)
}


pub fn find_last_version(tree: &breezyshim::tree::WorkingTree, cfg: &project_config::ProjectConfig) -> Result<(crate::version::Version, Option<Status>), Box<dyn std::error::Error>> {
    if tree.has_filename(Path::new("Cargo.toml")) {
        log::debug!("Reading version from Cargo.toml");
        return Ok((cargo::find_version(tree)?, None));
    }
    if tree.has_filename(Path::new("pyproject.toml")) {
        log::debug!("Reading version from pyproject.toml");
        if let Some(version) = python::find_version_in_pyproject_toml(tree) {
            return Ok((version, None));
        }
        if python::pyproject_uses_hatch_vcs(tree)? {
            let version = match python::find_hatch_vcs_version(tree) {
                Some(version) => version,
                None => {
                    unimplemented!("hatch in use but unable to find hatch vcs version");
                }
            };
            return Ok((version, None));
        }
    }
    for update_cfg in cfg.update_version.iter() {
        let path = match update_cfg.path.as_ref() {
            Some(path) => path,
            None => {
                warn!("update_version.path is required");
                continue;
            }
        };
        let new_line = match update_cfg.new_line.as_ref() {
            Some(new_line) => new_line,
            None => {
                warn!("update_version.new_line is required");
                continue;
            }
        };
        log::debug!("Reading version from {}", path);
        let f = tree.get_file(Path::new(path.as_str())).unwrap();
        use std::io::BufRead;
        let buf = std::io::BufReader::new(f);
        let lines = buf.lines().map(|l| l.unwrap()).collect::<Vec<_>>();
        let (v, s) = custom::reverse_version(new_line.as_str(), lines.iter().map(|l| l.as_str()).collect::<Vec<_>>().as_slice());
        if let Some(v) = v {
            return Ok((v, s));
        }
    }
    Err("Unable to find version".into())
}

#[derive(Debug)]
struct OddPendingVersion(String);

impl std::fmt::Display for OddPendingVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "OddPendingVersion: {}", self.0)
    }
}

impl std::error::Error for OddPendingVersion {}

pub fn find_pending_version(tree: &dyn breezyshim::tree::Tree, cfg: &project_config::ProjectConfig) -> Result<Option<Version>, Box<dyn std::error::Error>> {
    if let Some(news_file) = cfg.news_file.as_ref() {
        match news_file::news_find_pending(tree, Path::new(news_file.as_str())) {
            Ok(version) => Ok(version.map(|v| v.parse().unwrap())),
            Err(e) if e.downcast_ref::<news_file::OddVersion>().is_some() => {
                Err(Box::new(OddPendingVersion(e.to_string())))
            }
            Err(e) => Err(e),
        }
    } else {
        Ok(None)
    }
}
