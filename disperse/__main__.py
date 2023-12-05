#!/usr/bin/python3
# Copyright (C) 2021 Jelmer Vernooij <jelmer@jelmer.uk>
#
# This program is free software; you can redistribute it and/or modify
# it under the terms of the GNU General Public License as published by
# the Free Software Foundation; either version 2 of the License, or
# (at your option) any later version.
#
# This program is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU General Public License for more details.
#
# You should have received a copy of the GNU General Public License
# along with this program; if not, write to the Free Software
# Foundation, Inc., 51 Franklin Street, Fifth Floor, Boston, MA 02110-1301 USA

import logging
import subprocess
from datetime import datetime
from glob import glob
from typing import List, Optional, Tuple
from urllib.parse import urlparse

import breezy.bzr  # noqa: F401
import breezy.git  # noqa: F401
import breezy.plugins.launchpad  # noqa: F401
from breezy.branch import Branch
from breezy.git.remote import ProtectedBranchHookDeclined
from breezy.transport import NoSuchFile
from breezy.urlutils import split_segment_parameters
from prometheus_client import CollectorRegistry, Counter
from silver_platter.workspace import Workspace

from . import _disperse_rs
from . import NoUnreleasedChanges, DistCreationFailed
from .cargo import cargo_publish, update_version_in_cargo
from .project_config import read_project_with_fallback, ProjectConfig
from .github import (
    GitHubStatusFailed,
    GitHubStatusPending,
    check_gh_repo_action_status,
    get_github_repo,
    create_github_release,
    wait_for_gh_actions,
)
from .launchpad import add_release_files as add_launchpad_release_files
from .launchpad import create_milestone as create_launchpad_milestone
from .launchpad import ensure_release as ensure_launchpad_release
from .launchpad import get_project as get_launchpad_project
from .news_file import NewsFile
from .python import (
    UploadCommandFailed,
    create_python_artifacts,
    create_setup_py_artifacts,
    read_project_urls_from_setup_cfg,
    read_project_urls_from_pyproject_toml,
    upload_python_artifacts,
    update_version_in_pyproject_toml,
    find_name_in_pyproject_toml,
)

DEFAULT_CI_TIMEOUT = 7200


registry = CollectorRegistry()

ci_ignored_count = Counter(
    "ci_ignored",
    "CI was failing but ignored per user request",
    registry=registry,
    labelnames=["project"],
)

no_disperse_config = Counter(
    "no_disperse_config", "No disperse configuration present", registry=registry
)

recent_commits_count = Counter(
    "recent_commits",
    "There were recent commits, so no release was done",
    registry=registry,
    labelnames=["project"],
)

pre_dist_command_failed = Counter(
    "pre_dist_command_failed",
    "The pre-dist command failed to run",
    registry=registry,
    labelnames=["project"],
)


verify_command_failed = Counter(
    "verify_command_failed",
    "The verify command failed to run",
    registry=registry,
    labelnames=["project"],
)


branch_protected_count = Counter(
    "branch_protected",
    "The branch was protected",
    registry=registry,
    labelnames=["project"],
)


released_count = Counter(
    "released", "Released projects", registry=registry, labelnames=["project"]
)


no_unreleased_changes_count = Counter(
    "no_unreleased_changes",
    "There were no unreleased changes",
    registry=registry,
    labelnames=["project"],
)


release_tag_exists = Counter(
    "release_tag_exists",
    "A release tag already exists",
    registry=registry,
    labelnames=["project"],
)


class RepositoryUnavailable(Exception):
    """Indicates that a repository is unavailable."""


class RecentCommits(Exception):
    """Indicates there are too recent commits for a package."""

    def __init__(self, commit_age, min_commit_age):
        self.commit_age = commit_age
        self.min_commit_age = min_commit_age
        super().__init__(
            f"Last commit is only {self.commit_age} days old "
            f"(< {self.min_commit_age})"
        )


class VerifyCommandFailed(Exception):
    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


class PreDistCommandFailed(Exception):
    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


class NodisperseConfig(Exception):
    """No disperse config present"""


class ReleaseTagExists(Exception):
    def __init__(self, project, version, tag_name):
        self.project = project
        self.version = version
        self.tag_name = tag_name


increase_version = _disperse_rs.increase_version


class OddPendingVersion(Exception):
    """Indicates that the pending version is odd."""

    def __init__(self, version):
        self.version = version
        super().__init__(f"Pending version {self.version} is odd.")


find_pending_version = _disperse_rs.find_pending_version
version_line_re = _disperse_rs.version_line_re
expand_version_vars = _disperse_rs.expand_version_vars

update_version_in_file = _disperse_rs.update_version_in_file

update_version_in_manpage = _disperse_rs.update_version_in_manpage


def check_release_age(branch: Branch, cfg: ProjectConfig, now: datetime) -> None:
    rev = branch.repository.get_revision(branch.last_revision())
    if cfg.timeout_days is not None:
        commit_time = datetime.fromtimestamp(rev.timestamp)
        time_delta = now - commit_time
        if time_delta.days < cfg.timeout_days:
            raise RecentCommits(time_delta.days, cfg.timeout_days)


reverse_version = _disperse_rs.reverse_version
find_last_version = _disperse_rs.find_last_version


check_new_revisions = _disperse_rs.check_new_revisions

expand_tag = _disperse_rs.expand_tag
unexpand_tag = _disperse_rs.unexpand_tag


def release_project(  # noqa: C901
    repo_url: str,
    *,
    force: bool = False,
    new_version: Optional[str] = None,
    dry_run: bool = False,
    ignore_ci: bool = False,
):
    from breezy.controldir import ControlDir
    from breezy.transport.local import LocalTransport

    try:
        from breezy.errors import ConnectionError  # type: ignore
    except ImportError:
        ConnectionError = __builtins__['ConnectionError'] # type: ignore

    now = datetime.now()
    try:
        local_wt, branch = ControlDir.open_tree_or_branch(repo_url)
    except ConnectionError as e:
        raise RepositoryUnavailable(e)

    public_repo_url: Optional[str]

    if not isinstance(branch.user_transport, LocalTransport):
        public_repo_url = repo_url
        public_branch = branch
        local_branch = None
    elif branch.get_public_branch():
        public_repo_url = branch.get_public_branch()
        public_branch = Branch.open(public_repo_url)
        local_branch = branch
        logging.info("Using public branch %s", public_repo_url)
    elif branch.get_submit_branch() and not branch.get_submit_branch().startswith(
        "file:"
    ):
        public_repo_url = branch.get_submit_branch()
        public_branch = Branch.open(public_repo_url)
        local_branch = branch
        logging.info("Using public branch %s", public_repo_url)
    else:
        public_repo_url = branch.get_push_location()
        public_branch = Branch.open(public_repo_url)
        local_branch = branch
        logging.info("Using public branch %s", public_repo_url)

    if public_repo_url:
        public_repo_url = split_segment_parameters(public_repo_url)[0]
    else:
        public_repo_url = None

    if public_repo_url:
        logging.info("Found public repository URL: %s", public_repo_url)

    with Workspace(public_branch, resume_branch=local_branch) as ws:
        try:
            cfg = read_project_with_fallback(ws.local_tree)
        except NoSuchFile as exc:
            no_disperse_config.inc()
            raise NodisperseConfig() from exc

        name: Optional[str]
        if cfg.name:
            name = cfg.name
        elif ws.local_tree.has_filename("pyproject.toml"):
            name = find_name_in_pyproject_toml(ws.local_tree)
        else:
            name = None

        if cfg.launchpad_project:
            launchpad_project = get_launchpad_project(cfg.launchpad_project)
        else:
            launchpad_project = None

        if cfg.launchpad_series:
            launchpad_series = cfg.launchpad_series
        else:
            launchpad_series = None

        if cfg.github_url:
            gh_repo = get_github_repo(cfg.github_url)
            try:
                check_gh_repo_action_status(gh_repo, cfg.github_branch or "HEAD")
            except (GitHubStatusFailed, GitHubStatusPending) as e:
                if ignore_ci:
                    ci_ignored_count.labels(project=name).inc()
                    logging.warning("Ignoring failing CI: %s", e)
                else:
                    raise
        else:
            possible_urls = []
            if ws.local_tree.has_filename("setup.cfg"):
                possible_urls.extend(
                    read_project_urls_from_setup_cfg(ws.local_tree.abspath("setup.cfg"))
                )
            if ws.local_tree.has_filename("pyproject.toml"):
                possible_urls.extend(
                    read_project_urls_from_pyproject_toml(
                        ws.local_tree.abspath("pyproject.toml")
                    )
                )
            if public_repo_url is not None:
                possible_urls.append((public_repo_url, public_branch.name))

            for url, branch_name in possible_urls:
                parsed_url = urlparse(url)
                hostname = parsed_url.hostname
                if hostname == "github.com":
                    gh_repo = get_github_repo(url)
                    try:
                        check_gh_repo_action_status(gh_repo, branch_name)
                    except (GitHubStatusFailed, GitHubStatusPending) as e:
                        if ignore_ci:
                            logging.warning("Ignoring failing CI: %s", e)
                            ci_ignored_count.labels(project=name).inc()
                        else:
                            raise
                    break
                elif hostname == "launchpad.net":
                    parts = parsed_url.path.strip("/").split("/")[0]
                    launchpad_project = get_launchpad_project(parts[0])
                    if len(parts) > 1 and not parts[1].startswith("+"):
                        launchpad_series = parts[1]
            else:
                gh_repo = None

        if not check_new_revisions(ws.local_tree.branch, cfg.news_file):
            no_unreleased_changes_count.labels(project=name).inc()
            raise NoUnreleasedChanges()
        try:
            check_release_age(ws.local_tree.branch, cfg, now)
        except RecentCommits:
            recent_commits_count.labels(project=name).inc()
            if not force:
                raise
        if new_version is None:
            try:
                new_version = find_pending_version(ws.local_tree, cfg)
            except NotImplementedError:
                new_version = None
            if new_version is None:
                try:
                    last_version, last_version_status = find_last_version(
                        ws.local_tree, cfg
                    )
                except NotImplementedError:
                    last_version, last_version_status = find_last_version_in_tags(
                        ws.local_tree.branch, cfg.tag_name
                    )
                last_version_tag_name = expand_tag(cfg.tag_name, last_version)
                if ws.local_tree.branch.tags.has_tag(last_version_tag_name):
                    new_version = increase_version(last_version)
                else:
                    new_version = last_version
            assert new_version
            logging.info("Picked new version: %s", new_version)

        assert " " not in str(new_version), "Invalid version %r" % new_version

        if cfg.pre_dist_command:
            try:
                subprocess.check_call(
                    cfg.pre_dist_command, cwd=ws.local_tree.abspath("."), shell=True
                )
            except subprocess.CalledProcessError as e:
                pre_dist_command_failed.labels(project=name).inc()
                raise PreDistCommandFailed(cfg.pre_dist_command, e.returncode)

        if cfg.verify_command:
            verify_command = cfg.verify_command
        else:
            if ws.local_tree.has_filename("tox.ini"):
                verify_command = "tox"
            else:
                verify_command = None

        logging.info("releasing %s", new_version)
        news_file: Optional[NewsFile]
        if cfg.news_file:
            news_file = NewsFile(ws.local_tree, cfg.news_file)
            release_changes = news_file.mark_released(new_version, now)
        else:
            news_file = None
            release_changes = None
        for update_version in cfg.update_version:
            update_version_in_file(
                ws.local_tree,
                update_version.path,
                update_version.new_line,
                update_version.match,
                new_version,
                "final",
            )
        for update_manpage in cfg.update_manpages:
            for path in glob(ws.local_tree.abspath(update_manpage)):
                update_version_in_manpage(
                    ws.local_tree, ws.local_tree.relpath(path), new_version, now
                )
        if ws.local_tree.has_filename("Cargo.toml"):
            update_version_in_cargo(ws.local_tree, new_version)
        if ws.local_tree.has_filename("pyproject.toml"):
            update_version_in_pyproject_toml(ws.local_tree, new_version)
        ws.local_tree.commit(f"Release {new_version}.")

        if verify_command:
            try:
                subprocess.check_call(
                    verify_command, cwd=ws.local_tree.abspath("."), shell=True
                )
            except subprocess.CalledProcessError as e:
                verify_command_failed.labels(project=name).inc()
                raise VerifyCommandFailed(cfg.verify_command, e.returncode)

        tag_name = expand_tag(cfg.tag_name, new_version)
        if ws.main_branch.tags.has_tag(tag_name):
            release_tag_exists.labels(project=name).inc()
            # Maybe there's a pending pull request merging new_version?
            # TODO(jelmer): Do some more verification. Expect: release tag
            # has one additional revision that's not on our branch.
            raise ReleaseTagExists(name, new_version, tag_name)
        logging.info("Creating tag %s", tag_name)
        if hasattr(ws.local_tree.branch.repository, "_git"):
            subprocess.check_call(
                ["git", "tag", "-as", tag_name, "-m", f"Release {new_version}"],
                cwd=ws.local_tree.abspath("."),
            )
        else:
            ws.local_tree.branch.tags.set_tag(tag_name, ws.local_tree.last_revision())
        if ws.local_tree.has_filename("setup.py"):
            pypi_paths = create_setup_py_artifacts(ws.local_tree)
        elif ws.local_tree.has_filename("pyproject.toml"):
            pypi_paths = create_python_artifacts(ws.local_tree)
        else:
            pypi_paths = None

        artifacts = []
        if not dry_run:
            ws.push_tags(
                tags={tag_name: ws.local_tree.branch.tags.lookup_tag(tag_name)}
            )
        try:
            # Wait for CI to go green
            if gh_repo:
                if dry_run:
                    logging.info("In dry-run mode, so unable to wait for CI")
                else:
                    wait_for_gh_actions(
                        gh_repo,
                        tag_name,
                        timeout=(cfg.ci_timeout or DEFAULT_CI_TIMEOUT),
                    )

            if pypi_paths:
                artifacts.extend(pypi_paths)
                if dry_run:
                    logging.info("skipping twine upload due to dry run mode")
                elif cfg.skip_twine_upload:
                    logging.info("skipping twine upload; disabled in config")
                else:
                    upload_python_artifacts(ws.local_tree, pypi_paths)
            if ws.local_tree.has_filename("Cargo.toml"):
                if dry_run:
                    logging.info("skipping cargo upload due to dry run mode")
                else:
                    cargo_publish(ws.local_tree, ".")
            for loc in cfg.tarball_location:
                if dry_run:
                    logging.info("skipping scp to %s due to dry run mode", loc)
                else:
                    subprocess.check_call(["scp"] + artifacts + [loc])
        except BaseException:
            logging.info("Deleting remote tag %s", tag_name)
            if not dry_run:
                ws.main_branch.tags.delete_tag(tag_name)
            raise

        # At this point, it's official - so let's push.
        try:
            if not dry_run:
                ws.push()
        except ProtectedBranchHookDeclined:
            branch_protected_count.labels(project=name).inc()
            logging.info(
                "branch %s is protected; proposing merge instead",
                ws.local_tree.branch.name,
            )
            if not dry_run:
                (mp, _is_new) = ws.propose(
                    description=f"Merge release of {new_version}",
                    tags=[tag_name],
                    name=f"release-{new_version}",
                    labels=["release"],
                    commit_message=f"Merge release of {new_version}",
                )
            else:
                mp = None
            logging.info(f"Created merge proposal: {mp.url}")

            if getattr(mp, "supports_auto_merge", False):
                mp.merge(auto=True, message=f"Merge release of {new_version}")

        if gh_repo:
            if dry_run:
                logging.info("skipping creation of github release due to dry run mode")
            else:
                create_github_release(gh_repo, tag_name, new_version, release_changes)

        if launchpad_project:
            if dry_run:
                logging.info("skipping upload of tarball to Launchpad")
            else:
                lp_release = ensure_launchpad_release(
                    launchpad_project,
                    new_version,
                    series_name=launchpad_series,
                    release_notes=release_changes,
                )
                add_launchpad_release_files(lp_release, artifacts)

        # TODO(jelmer): Mark any news bugs in NEWS as fixed [later]
        # * Commit:
        #  * Update NEWS and version strings for next version
        new_pending_version = increase_version(new_version, -1)
        logging.info("Using new version %s", new_pending_version)
        if news_file:
            news_file.add_pending(new_pending_version)
            ws.local_tree.commit(f"Start on {new_pending_version}")
            if not dry_run:
                ws.push()
        if launchpad_project:
            if dry_run:
                logging.info(
                    "Skipping creation of new milestone %s on Launchpad",
                    new_pending_version,
                )
            else:
                create_launchpad_milestone(
                    launchpad_project, new_pending_version, series_name=launchpad_series
                )
    if not dry_run:
        if local_wt is not None:
            local_wt.pull(public_branch)
        elif local_branch is not None:
            local_branch.pull(public_branch)

    released_count.labels(project=name).inc()
    return name, new_version


find_last_version_in_tags = _disperse_rs.find_last_version_in_tags


def release_many(
    urls,
    *,
    force=False,
    dry_run=False,
    discover=False,  # noqa: C901
    new_version=None,
    ignore_ci=False,
):
    failed: List[Tuple[str, Exception]] = []
    skipped: List[Tuple[str, Exception]] = []
    success = []
    ret = 0
    for url in urls:
        if url != ".":
            logging.info("Processing %s", url)
        try:
            release_project(
                url,
                force=force,
                new_version=new_version,
                dry_run=dry_run,
                ignore_ci=ignore_ci,
            )
        except RecentCommits as e:
            logging.info(
                "Recent commits exist (%d < %d)", e.min_commit_age, e.commit_age
            )
            skipped.append((url, e))
            if not discover:
                ret = 1
        except VerifyCommandFailed as e:
            logging.error("Verify command (%s) failed to run.", e.command)
            failed.append((url, e))
            ret = 1
        except PreDistCommandFailed as e:
            logging.error("Pre-Dist command (%s) failed to run.", e.command)
            failed.append((url, e))
            ret = 1
        except UploadCommandFailed as e:
            logging.error("Upload command (%s) failed to run.", e.args[0])
            failed.append((url, e))
            ret = 1
        except ReleaseTagExists as e:
            logging.warning(
                "%s: Release tag %s for version %s exists. " "Unmerged release commit?",
                e.project,
                e.tag_name,
                e.version,
            )
            skipped.append((url, e))
            if not discover:
                ret = 1
        except DistCreationFailed as e:
            logging.error("Dist creation failed to run: %s", e)
            failed.append((url, e))
            ret = 1
        except NoUnreleasedChanges as e:
            logging.error("No unreleased changes")
            skipped.append((url, e))
            if not discover:
                ret = 1
        except NodisperseConfig as e:
            logging.error("No configuration for disperse")
            skipped.append((url, e))
            if not discover:
                ret = 1
        except GitHubStatusPending as e:
            logging.error(
                "GitHub checks for commit %s " "not finished yet. See %s",
                e.sha,
                e.html_url,
            )
            failed.append((url, e))
            ret = 1
        except GitHubStatusFailed as e:
            logging.error(
                "GitHub check for commit %s failed. " "See %s", e.sha, e.html_url
            )
            failed.append((url, e))
            ret = 1
        except RepositoryUnavailable as e:
            logging.error("Repository is unavailable: %s", e.args[0])
            failed.append((url, e))
            ret = 1
        except OddPendingVersion as e:
            logging.error("Odd pending version: %s", e.version)
            failed.append((url, e))
            ret = 1
        else:
            success.append(url)

    if discover:
        logging.info(
            "%s successfully released, %s skipped, %s failed",
            len(success),
            len(skipped),
            len(failed),
        )
    return ret
