use breezyshim::tree::Tree;
use clap::Parser;
use maplit::hashmap;
use pyo3::prelude::*;
use std::io::Write;
use url::Url;

use prometheus::{default_registry, Encoder, TextEncoder};

fn push_to_gateway(prometheus_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut buffer = vec![];
    let encoder = TextEncoder::new();
    encoder.encode(&default_registry().gather(), &mut buffer)?;

    let metrics = String::from_utf8(buffer)?;

    let url = format!("{}/metrics/job/disperse", prometheus_url);
    reqwest::blocking::Client::new()
        .post(url)
        .body(metrics)
        .send()?
        .error_for_status()?;

    Ok(())
}

#[derive(Parser)]
struct Args {
    #[clap(long)]
    debug: bool,

    #[clap(long)]
    dry_run: bool,

    #[clap(long)]
    prometheus: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    Release(ReleaseArgs),
    Discover(DiscoverArgs),
    Validate(ValidateArgs),
    Info(InfoArgs),
}

#[derive(clap::Args)]
struct ReleaseArgs {
    #[clap(default_value = ".")]
    url: Vec<Url>,

    /// New version to release
    #[clap(long)]
    new_version: Option<String>,

    /// Release, even if the CI is not passing
    #[clap(long)]
    ignore_ci: bool,
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
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

#[derive(clap::Args)]
struct InfoArgs {
    #[clap(default_value = ".")]
    path: std::path::PathBuf,
}

fn info(
    tree: &dyn breezyshim::tree::Tree,
    branch: &dyn breezyshim::branch::Branch,
) -> pyo3::PyResult<i32> {
    pyo3::Python::with_gil(|py| {
        let m = py.import("disperse.__main__")?;
        let info = m.getattr("info")?;
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("tree", tree)?;
        kwargs.set_item("branch", branch)?;
        Ok(info
            .call((), Some(kwargs))?
            .extract::<Option<i32>>()?
            .unwrap_or(0))
    })
}

fn info_many(urls: &[Url]) -> pyo3::PyResult<i32> {
    let mut ret = 0;

    for url in urls {
        if url.to_string() != "." {
            log::info!("Processing {}", url);
        }

        let (local_wt, branch) =
            match breezyshim::controldir::ControlDir::open_tree_or_branch(url, None) {
                Ok(x) => x,
                Err(e) => {
                    ret = 1;
                    log::error!("Unable to open {}: {}", url, e);
                    continue;
                }
            };

        if let Some(wt) = local_wt {
            let lock = wt.lock_read();
            ret += info(&wt, wt.branch().as_ref()).unwrap_or(0);
            std::mem::drop(lock);
        } else {
            let lock = branch.lock_read().unwrap();
            match info(&branch.basis_tree().unwrap(), branch.as_ref()) {
                Ok(_) => {
                    std::mem::drop(lock);
                }
                Err(e) => {
                    // TODO(jelmer): Just handle UnsupporedOperation
                    let ws = silver_platter::workspace::Workspace::from_url(
                        url,
                        None,
                        None,
                        hashmap! {},
                        hashmap! {},
                        None,
                        None,
                        None,
                    );
                    let lock = ws.local_tree().lock_read();
                    ret += info(&ws.local_tree(), ws.local_tree().branch().as_ref()).unwrap_or(0);
                    std::mem::drop(lock);
                }
            }
        }
    }
    Ok(ret)
}

fn release_many(
    urls: &[Url],
    new_version: Option<String>,
    ignore_ci: bool,
    dry_run: bool,
) -> pyo3::PyResult<i32> {
    pyo3::Python::with_gil(|py| {
        let m = py.import("disperse.__main__")?;
        let release_many = m.getattr("release_many")?;
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item(
            "urls",
            urls.iter().map(|u| u.to_string()).collect::<Vec<_>>(),
        )?;
        kwargs.set_item("force", true)?;
        kwargs.set_item("dry_run", dry_run)?;
        kwargs.set_item("discover", false)?;
        kwargs.set_item("new_version", new_version)?;
        kwargs.set_item("ignore_ci", ignore_ci)?;
        release_many
            .call((), Some(kwargs))?
            .extract::<Option<i32>>()
            .map(|x| x.unwrap_or(0))
    })
}

fn validate_config(path: &std::path::Path) -> pyo3::PyResult<i32> {
    pyo3::Python::with_gil(|py| {
        let m = py.import("disperse.__main__")?;
        let validate_config = m.getattr("validate_config")?;
        let kwargs = pyo3::types::PyDict::new(py);
        kwargs.set_item("path", path)?;
        validate_config
            .call((), Some(kwargs))?
            .extract::<Option<i32>>()
            .map(|x| x.unwrap_or(0))
    })
}

fn main() {
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

    breezyshim::init().unwrap();

    std::process::exit(match &args.command {
        Commands::Release(release_args) => release_many(
            release_args.url.as_slice(),
            release_args.new_version.clone(),
            release_args.ignore_ci,
            args.dry_run,
        )
        .unwrap(),
        Commands::Discover(discover_args) => {
            let pypi_usernames = match discover_args.pypi_user.as_slice() {
                [] => config
                    .pypi
                    .map(|pypi| vec![pypi.username])
                    .unwrap_or(vec![]),
                pypi_usernames => pypi_usernames.to_vec(),
            };

            let crates_io_user = match discover_args.crates_io_user.as_ref() {
                None => config.crates_io.map(|crates_io| crates_io.username),
                Some(crates_io_user) => Some(crates_io_user.clone()),
            };

            let pypi_urls = pypi_usernames
                .iter()
                .flat_map(|pypi_username| disperse::python::pypi_discover_urls(pypi_username))
                .flatten()
                .collect::<Vec<_>>();

            let crates_io_urls = match crates_io_user {
                None => {
                    vec![]
                }
                Some(crates_io_user) => {
                    disperse::cargo::get_owned_crates(crates_io_user.as_str()).unwrap()
                }
            };

            let repositories_urls = config
                .repositories
                .and_then(|repositories| repositories.owned)
                .unwrap_or(vec![]);

            let urls: Vec<Url> = vec![pypi_urls, crates_io_urls, repositories_urls]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>();

            if urls.is_empty() {
                log::error!("No projects found. Specify pypi or crates.io username, or add repositories to config");
                0
            } else {
                let ret = if discover_args.info {
                    info_many(urls.as_slice()).unwrap()
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
                    release_many(urls.as_slice(), None, false, false).unwrap()
                };
                if let Some(prometheus) = args.prometheus {
                    push_to_gateway(prometheus.as_str()).unwrap();
                }
                if discover_args.r#try {
                    0
                } else {
                    ret
                }
            }
        }
        Commands::Validate(args) => validate_config(&args.path).unwrap(),
        Commands::Info(args) => {
            let wt = breezyshim::tree::WorkingTree::open(args.path.as_ref()).unwrap();
            info(&wt, wt.branch().as_ref()).unwrap()
        }
    });
}
