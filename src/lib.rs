pub mod cargo;
pub mod project_config;
pub mod update_version;
pub mod version;
use breezyshim::branch::Branch;
pub use version::Version;

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
