use breezyshim::tree::{Error as TreeError, Tree};
use std::path::Path;
include!(concat!(env!("OUT_DIR"), "/generated/mod.rs"));

pub use config::Project as ProjectConfig;
pub use config::UpdateVersion;

fn read_project(f: &mut dyn std::io::Read) -> ProjectConfig {
    let mut s = String::new();
    std::io::Read::read_to_string(f, &mut s).unwrap();

    let ret: ProjectConfig = protobuf::text_format::parse_from_str(&s).unwrap();
    ret
}

pub fn read_project_with_fallback(tree: &dyn Tree) -> Result<ProjectConfig, TreeError> {
    let mut f = match tree.get_file(Path::new("disperse.conf")) {
        Ok(f) => f,
        Err(TreeError::NoSuchFile(f)) => match tree.get_file(Path::new("releaser.conf")) {
            Err(_) => return Err(TreeError::NoSuchFile(f)),
            Ok(f) => f,
        },
        Err(e) => return Err(e),
    };

    Ok(read_project(&mut f))
}
