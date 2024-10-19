use launchpadlib::v1_0::{
    Milestone, Project, ProjectFull, ProjectRelease, ProjectReleaseDiff, ProjectReleaseFull,
    ProjectSeriesFull,
};
use launchpadlib::Client;

pub fn find_project_series(
    client: &Client,
    project: &Project,
    series_name: Option<&str>,
    target_version: Option<&str>,
) -> Result<ProjectSeriesFull, String> {
    let project = project
        .get(client)
        .map_err(|e| format!("Failed to get project: {}", e))?;
    let mut series = project
        .series(client)
        .map_err(|e| format!("Failed to get series: {}", e))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to get series: {}", e))?;

    if let Some(series_name) = series_name {
        series
            .into_iter()
            .find(|s| s.name == series_name)
            .ok_or_else(|| format!("No series named {} found", series_name))
    } else if series.len() == 1 {
        Ok(series.pop().unwrap())
    } else if series.len() > 1 {
        let mut possible_series = series
            .into_iter()
            .filter(|s| {
                if let Some(target_version) = target_version {
                    target_version.starts_with(s.name.as_str())
                } else {
                    s.active
                }
            })
            .collect::<Vec<_>>();
        if possible_series.len() == 1 {
            return Ok(possible_series.pop().unwrap());
        } else {
            log::warn!(
                "Multiple release series exist, but none specified. Assuming development focus"
            );
            return project
                .development_focus()
                .get(client)
                .map_err(|e| format!("Failed to get development focus: {}", e));
        }
    } else {
        panic!("no release series for {:?}", project);
    }
}

pub fn create_milestone(
    client: &Client,
    project: &Project,
    version: &str,
    series_name: Option<&str>,
) -> Result<Milestone, String> {
    let series = find_project_series(client, project, series_name, None)?;
    let release_date = chrono::Utc::now().date().naive_utc();
    Ok(series
        .self_()
        .unwrap()
        .new_milestone(client, version, Some(&release_date), None, None)
        .map_err(|e| format!("Failed to create milestone: {}", e))?
        .unwrap())
}

pub fn get_project(client: &Client, project: &str) -> Result<ProjectFull, String> {
    let root = launchpadlib::v1_0::service_root(client)
        .map_err(|e| format!("Failed to get service root: {}", e))?;

    // Look up the project using the Launchpad instance.
    let projects = root
        .projects()
        .unwrap()
        .iter(client)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    projects
        .into_iter()
        .find(|p| p.name == project)
        .ok_or_else(|| format!("No project named {} found", project))
}

pub fn find_release(
    client: &Client,
    project: &Project,
    release: &str,
) -> Option<ProjectReleaseFull> {
    let project = project.get(client).unwrap();
    let releases = project
        .releases(client)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    releases
        .into_iter()
        .find(|r| r.version.to_string() == release)
}

pub fn create_release_from_milestone(
    client: &Client,
    project: &Project,
    version: &str,
) -> Option<ProjectRelease> {
    let project = project.get(client).unwrap();
    for milestone in project.all_milestones(client).unwrap() {
        let milestone = milestone.unwrap();
        if milestone.name == version {
            let today = chrono::Utc::now();
            return Some(
                milestone
                    .self_()
                    .unwrap()
                    .create_product_release(client, &today, None, None)
                    .unwrap()
                    .unwrap(),
            );
        }
    }
    None
}

pub fn ensure_release(
    client: &Client,
    proj: &Project,
    version: &str,
    series_name: Option<&str>,
    release_notes: Option<&str>,
) -> Result<ProjectRelease, String> {
    if let Some(release) = find_release(client, proj, version) {
        let release = release.self_().unwrap();
        let diff = ProjectReleaseDiff {
            release_notes: release_notes.map(|s| s.to_string()),
            ..Default::default()
        };

        release
            .patch(client, &diff)
            .map_err(|e| format!("Failed to update release: {}", e))?;
        Ok(release)
    } else if let Some(release) = create_release_from_milestone(client, proj, version) {
        let diff = ProjectReleaseDiff {
            release_notes: release_notes.map(|s| s.to_string()),
            ..Default::default()
        };
        release
            .patch(client, &diff)
            .map_err(|e| format!("Failed to update release: {}", e))?;
        Ok(release)
    } else {
        let milestone = create_milestone(client, proj, version, series_name)?;
        let today = chrono::Utc::now();
        Ok(milestone
            .create_product_release(client, &today, None, release_notes)
            .map_err(|e| format!("Failed to create release: {}", e))?
            .unwrap())
    }
}

pub fn add_release_files(
    client: &Client,
    release: &ProjectRelease,
    artifacts: Vec<std::path::PathBuf>,
) -> Result<(), String> {
    for artifact in artifacts {
        if artifact.ends_with(".tar.gz") {
            release
                .add_file(
                    client,
                    Some("release tarball"),
                    artifact.file_name().unwrap().to_str().unwrap(),
                    None,
                    "application/x-gzip",
                    reqwest::blocking::multipart::Part::file(&artifact).unwrap(),
                    None,
                    Some(&launchpadlib::v1_0::FileType::CodeReleaseTarball),
                )
                .map_err(|e| format!("Failed to add release file: {}", e))
                .unwrap();
        }
    }
    Ok(())
}
