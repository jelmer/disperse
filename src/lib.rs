pub mod cargo;
pub mod project_config;
pub mod update_version;
pub mod version;
use breezyshim::branch::Branch;
use log::warn;
use project_config::Project;
pub use version::Version;

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
