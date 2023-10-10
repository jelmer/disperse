pub mod cargo;
use breezyshim::branch::Branch;
#[cfg(feature = "pyo3")]
use pyo3::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(String);

#[cfg(feature = "pyo3")]
impl ToPyObject for Version {
    fn to_object(&self, py: pyo3::Python) -> pyo3::PyObject {
        self.0.to_object(py)
    }
}

#[cfg(feature = "pyo3")]
impl IntoPy<pyo3::PyObject> for Version {
    fn into_py(self, py: pyo3::Python) -> pyo3::PyObject {
        self.0.into_py(py)
    }
}

#[cfg(feature = "pyo3")]
impl FromPyObject<'_> for Version {
    fn extract(ob: &pyo3::PyAny) -> pyo3::PyResult<Self> {
        let s = ob.extract::<String>()?;
        Ok(Version(s))
    }
}

pub fn expand_tag(tag_template: &str, version: Version) -> String {
    tag_template.replace("$VERSION", version.0.as_str())
}

pub fn increase_version(version: &mut Version, idx: isize) {
    // Split the version string by '.' and collect each part into a Vec<i32>
    // The `unwrap_or(0)` is there to handle the case where the string is not a valid integer.
    let mut parts: Vec<i32> = version
        .0
        .split('.')
        .map(|x| x.parse::<i32>().unwrap_or(0))
        .collect();

    // Calculate the index to modify.
    // We use `wrapping_add` to gracefully handle negative indices by making them wrap around to the end.
    let idx = ((idx.wrapping_add(parts.len() as isize)) as usize) % parts.len();

    parts[idx] += 1;

    // Convert each element back to a String and join them with '.'.
    // Return the resulting String.
    version.0 = parts
        .into_iter()
        .map(|x| x.to_string())
        .collect::<Vec<String>>()
        .join(".");
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
