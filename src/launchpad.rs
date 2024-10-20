use futures::TryStreamExt;
use launchpadlib::r#async::v1_0::{
    Milestone, Project, ProjectFull, ProjectRelease, ProjectReleaseDiff, ProjectReleaseFull,
    ProjectSeriesFull,
};
use launchpadlib::r#async::Client;

pub async fn find_project_series(
    client: &Client,
    project: &Project,
    series_name: Option<&str>,
    target_version: Option<&str>,
) -> Result<ProjectSeriesFull, String> {
    let project = project
        .get(client)
        .await
        .map_err(|e| format!("Failed to get project: {}", e))?;
    let mut series = project
        .series(client)
        .await
        .map_err(|e| format!("Failed to get series: {}", e))?
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

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
                .await
                .map_err(|e| format!("Failed to get development focus: {}", e));
        }
    } else {
        panic!("no release series for {:?}", project);
    }
}

pub async fn create_milestone(
    client: &Client,
    project: &Project,
    version: &str,
    series_name: Option<&str>,
) -> Result<Milestone, String> {
    let series = find_project_series(client, project, series_name, None).await?;
    let release_date = chrono::Utc::now().date_naive();
    Ok(series
        .self_()
        .unwrap()
        .new_milestone(client, version, Some(&release_date), None, None)
        .await
        .map_err(|e| format!("Failed to create milestone: {}", e))?
        .unwrap())
}

pub async fn get_project(client: &Client, project: &str) -> Result<ProjectFull, String> {
    let root = launchpadlib::r#async::v1_0::service_root(client)
        .await
        .map_err(|e| format!("Failed to get service root: {}", e))?;

    // Look up the project using the Launchpad instance.
    let projects = root
        .projects()
        .unwrap()
        .iter(client)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    projects
        .into_iter()
        .find(|p| p.name == project)
        .ok_or_else(|| format!("No project named {} found", project))
}

pub async fn find_release(
    client: &Client,
    project: &Project,
    release: &str,
) -> Option<ProjectReleaseFull> {
    let project = project.get(client).await.unwrap();
    let releases = project
        .releases(client)
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();

    releases
        .into_iter()
        .find(|r| r.version.to_string() == release)
}

pub async fn create_release_from_milestone(
    client: &Client,
    project: &Project,
    version: &str,
) -> Option<ProjectRelease> {
    let project = project.get(client).await.unwrap();

    let mut milestones = project.all_milestones(client).await.unwrap();

    while let Some(milestone) = milestones.try_next().await.unwrap() {
        if milestone.name == version {
            let today = chrono::Utc::now();
            return Some(
                milestone
                    .self_()
                    .unwrap()
                    .create_product_release(client, &today, None, None)
                    .await
                    .unwrap()
                    .unwrap(),
            );
        }
    }
    None
}

pub async fn ensure_release(
    client: &Client,
    proj: &Project,
    version: &str,
    series_name: Option<&str>,
    release_notes: Option<&str>,
) -> Result<ProjectRelease, String> {
    if let Some(release) = find_release(client, proj, version).await {
        let release = release.self_().unwrap();
        let diff = ProjectReleaseDiff {
            release_notes: release_notes.map(|s| s.to_string()),
            ..Default::default()
        };

        release
            .patch(client, &diff)
            .await
            .map_err(|e| format!("Failed to update release: {}", e))?;
        Ok(release)
    } else if let Some(release) = create_release_from_milestone(client, proj, version).await {
        let diff = ProjectReleaseDiff {
            release_notes: release_notes.map(|s| s.to_string()),
            ..Default::default()
        };
        release
            .patch(client, &diff)
            .await
            .map_err(|e| format!("Failed to update release: {}", e))?;
        Ok(release)
    } else {
        let milestone = create_milestone(client, proj, version, series_name).await?;
        let today = chrono::Utc::now();
        Ok(milestone
            .create_product_release(client, &today, None, release_notes)
            .await
            .map_err(|e| format!("Failed to create release: {}", e))?
            .unwrap())
    }
}

pub async fn add_release_files(
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
                    reqwest::multipart::Part::file(&artifact).await.unwrap(),
                    None,
                    Some(&launchpadlib::r#async::v1_0::FileType::CodeReleaseTarball),
                )
                .await
                .map_err(|e| format!("Failed to add release file: {}", e))
                .unwrap();
        }
    }
    Ok(())
}
