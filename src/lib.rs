pub mod cargo;
pub mod config;
pub mod custom;
pub mod github;
pub mod launchpad;
pub mod manpage;
pub mod news_file;
pub mod project_config;
pub mod python;
pub mod version;
use breezyshim::branch::Branch;
use breezyshim::tree::Tree;
use breezyshim::workingtree::WorkingTree;
use log::warn;
use std::path::{Path, PathBuf};

pub use version::Version;

pub const USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Final,
    Dev,
}

#[cfg(feature = "pyo3")]
impl pyo3::FromPyObject<'_> for Status {
    fn extract_bound(ob: &pyo3::Bound<pyo3::PyAny>) -> pyo3::PyResult<Self> {
        use pyo3::prelude::*;
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
    let from_revid = graph
        .iter_lefthand_ancestry(&branch.last_revision(), None)
        .find_map(|revid| {
            let revid = revid.ok()?;
            if tags.contains_key(&revid) {
                Some(revid)
            } else {
                None
            }
        });

    log::debug!(
        "Checking revisions between {} and {}",
        branch.last_revision(),
        from_revid
            .as_ref()
            .map(|r| r.to_string())
            .unwrap_or_else(|| "null".to_string())
    );

    if from_revid == Some(branch.last_revision()) {
        return Ok(false);
    }

    let from_tree = from_revid
        .map(|r| repository.revision_tree(&r))
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
) -> Result<(Option<Version>, Option<Status>), Box<dyn std::error::Error>> {
    let rev_tag_dict = branch.tags()?.get_reverse_tag_dict()?;
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
        return Ok((Some(release), Some(status)));
    }

    warn!("Unable to find any tags matching {}", tag_name);
    Ok((None, None))
}

pub fn find_last_version_in_files(
    tree: &WorkingTree,
    cfg: &project_config::ProjectConfig,
) -> Result<Option<(crate::version::Version, Option<Status>)>, Box<dyn std::error::Error>> {
    if tree.has_filename(Path::new("Cargo.toml")) {
        log::debug!("Reading version from Cargo.toml");
        return Ok(Some((cargo::find_version(tree)?, None)));
    }
    if tree.has_filename(Path::new("pyproject.toml")) {
        log::debug!("Reading version from pyproject.toml");
        if let Some(version) = python::find_version_in_pyproject_toml(tree)? {
            return Ok(Some((version, None)));
        }
        if python::pyproject_uses_hatch_vcs(tree)? {
            let version = if let Some(version) = python::find_hatch_vcs_version(tree) {
                version
            } else {
                unimplemented!("hatch in use but unable to find hatch vcs version");
            };
            return Ok(Some((version, None)));
        }
    }
    for update_cfg in cfg.update_version.iter() {
        let path = &update_cfg.path;
        let new_line = &update_cfg.new_line;
        log::debug!("Reading version from {}", path.display());
        let f = tree.get_file(path).unwrap();
        use std::io::BufRead;
        let buf = std::io::BufReader::new(f);
        let lines = buf.lines().map(|l| l.unwrap()).collect::<Vec<_>>();
        let (v, s) = custom::reverse_version(
            new_line.as_str(),
            lines
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .as_slice(),
        );
        if let Some(v) = v {
            return Ok(Some((v, s)));
        }
    }
    Ok(None)
}

#[derive(Debug)]
pub enum FindPendingVersionError {
    OddPendingVersion(String),
    NoUnreleasedChanges,
    Other(Box<dyn std::error::Error>),
    NotFound,
}

impl std::fmt::Display for FindPendingVersionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OddPendingVersion(e) => {
                write!(f, "Odd pending version: {}", e)
            }
            Self::NotFound => {
                write!(f, "No pending version found")
            }
            Self::Other(e) => {
                write!(f, "Other error: {}", e)
            }
            Self::NoUnreleasedChanges => {
                write!(f, "No unreleased changes")
            }
        }
    }
}

impl std::error::Error for FindPendingVersionError {}

pub fn find_pending_version(
    tree: &dyn breezyshim::tree::Tree,
    cfg: &project_config::ProjectConfig,
) -> Result<Version, FindPendingVersionError> {
    if let Some(news_file) = cfg.news_file.as_ref() {
        match news_file::news_find_pending(tree, news_file) {
            Ok(Some(version)) => Ok(version.parse().unwrap()),
            Ok(None) => Err(FindPendingVersionError::NoUnreleasedChanges),
            Err(news_file::Error::OddVersion(e)) => {
                Err(FindPendingVersionError::OddPendingVersion(e))
            }
            Err(news_file::Error::PendingExists { .. }) => {
                unreachable!();
            }
            Err(e) => Err(FindPendingVersionError::Other(Box::new(e))),
        }
    } else {
        Err(FindPendingVersionError::NotFound)
    }
}

pub fn drop_segment_parameters(u: &url::Url) -> url::Url {
    breezyshim::urlutils::split_segment_parameters(
        &u.as_str().trim_end_matches('/').parse().unwrap(),
    )
    .0
}

#[test]
fn test_drop_segment_parameters() {
    assert_eq!(
        drop_segment_parameters(&"https://example.com/foo/bar,baz=quux".parse().unwrap()),
        "https://example.com/foo/bar".parse().unwrap()
    );
    assert_eq!(
        drop_segment_parameters(&"https://example.com/foo/bar,baz=quux#frag".parse().unwrap()),
        "https://example.com/foo/bar".parse().unwrap()
    );
    assert_eq!(
        drop_segment_parameters(
            &"https://example.com/foo/bar,baz=quux#frag?frag2"
                .parse()
                .unwrap()
        ),
        "https://example.com/foo/bar".parse().unwrap()
    );
}

pub fn iter_glob<'a>(
    local_tree: &'a WorkingTree,
    pattern: &str,
) -> impl Iterator<Item = PathBuf> + 'a {
    let abspath = local_tree.basedir();

    glob::glob(format!("{}/{}", abspath.to_str().unwrap(), pattern).as_str())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|path| local_tree.relpath(path.as_path()).unwrap())
        .filter(|p| !local_tree.is_control_filename(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_iter_glob() {
        let td = tempfile::tempdir().unwrap();
        let local_tree = breezyshim::controldir::create_standalone_workingtree(
            td.path(),
            &breezyshim::controldir::ControlDirFormat::default(),
        )
        .unwrap();
        std::fs::write(local_tree.basedir().join("foo"), "").unwrap();
        std::fs::write(local_tree.basedir().join("bar"), "").unwrap();
        assert_eq!(
            iter_glob(&local_tree, "*").collect::<Vec<_>>(),
            vec![PathBuf::from("bar"), PathBuf::from("foo")]
        );
        assert_eq!(
            iter_glob(&local_tree, "foo").collect::<Vec<_>>(),
            vec![PathBuf::from("foo")]
        );
        assert_eq!(
            iter_glob(&local_tree, "bar").collect::<Vec<_>>(),
            vec![PathBuf::from("bar")]
        );
        assert_eq!(
            iter_glob(&local_tree, "baz").collect::<Vec<_>>(),
            Vec::<PathBuf>::new()
        );
        assert_eq!(
            iter_glob(&local_tree, "*o").collect::<Vec<_>>(),
            vec![PathBuf::from("foo")]
        );
    }
}
