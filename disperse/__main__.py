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
import re
import subprocess
from datetime import datetime
from glob import glob
from typing import List, Optional, Tuple, Callable
from urllib.parse import urlparse

import breezy.bzr  # noqa: F401
import breezy.git  # noqa: F401
import breezy.plugins.launchpad  # noqa: F401
from breezy.branch import Branch
from breezy.errors import NoSuchTag
from breezy.git.remote import ProtectedBranchHookDeclined
from breezy.mutabletree import MutableTree
from breezy.revision import NULL_REVISION
from breezy.transport import NoSuchFile
from breezy.tree import Tree
from breezy.urlutils import split_segment_parameters
from breezy.workingtree import WorkingTree
from prometheus_client import CollectorRegistry, Counter
from silver_platter.workspace import Workspace

from . import _disperse_rs
from . import NoUnreleasedChanges, DistCreationFailed
from .cargo import (
    cargo_publish, update_version_in_cargo, find_version_in_cargo)
from .project_config import read_project_with_fallback, ProjectConfig
from .github import (GitHubStatusFailed, GitHubStatusPending,
                     check_gh_repo_action_status, get_github_repo,
                     create_github_release, wait_for_gh_actions)
from .launchpad import add_release_files as add_launchpad_release_files
from .launchpad import create_milestone as create_launchpad_milestone
from .launchpad import ensure_release as ensure_launchpad_release
from .launchpad import get_project as get_launchpad_project
from .news_file import NewsFile, news_find_pending, OddVersion as OddNewsVersion
from .python import (UploadCommandFailed,
                     create_python_artifacts, create_setup_py_artifacts,
                     read_project_urls_from_setup_cfg,
                     read_project_urls_from_pyproject_toml,
                     upload_python_artifacts,
                     find_version_in_pyproject_toml, update_version_in_pyproject_toml,
                     pyproject_uses_hatch_vcs, find_hatch_vcs_version,
                     find_name_in_pyproject_toml,
                     )

DEFAULT_CI_TIMEOUT = 7200


registry = CollectorRegistry()

ci_ignored_count = Counter(
    'ci_ignored',
    'CI was failing but ignored per user request',
    registry=registry,
    labelnames=['project'])

no_disperse_config = Counter(
    'no_disperse_config',
    'No disperse configuration present',
    registry=registry)

recent_commits_count = Counter(
    'recent_commits',
    'There were recent commits, so no release was done',
    registry=registry,
    labelnames=['project'])

pre_dist_command_failed = Counter(
    'pre_dist_command_failed',
    'The pre-dist command failed to run',
    registry=registry,
    labelnames=['project'])


verify_command_failed = Counter(
    'verify_command_failed',
    'The verify command failed to run',
    registry=registry,
    labelnames=['project'])


branch_protected_count = Counter(
    'branch_protected',
    'The branch was protected',
    registry=registry,
    labelnames=['project'])


released_count = Counter(
    'released',
    'Released projects',
    registry=registry,
    labelnames=['project'])


no_unreleased_changes_count = Counter(
    'no_unreleased_changes',
    'There were no unreleased changes',
    registry=registry,
    labelnames=['project'])


release_tag_exists = Counter(
    'release_tag_exists',
    'A release tag already exists',
    registry=registry,
    labelnames=['project'])


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
        super().__init__(
            f"Pending version {self.version} is odd."
        )


def find_pending_version(tree: Tree, cfg) -> Optional[str]:
    if cfg.news_file:
        try:
            return news_find_pending(tree, cfg.news_file)
        except OddNewsVersion as e:
            raise OddPendingVersion(e.version) from e
    else:
        raise NotImplementedError


def _status_tupled_version(v, s):
    return "(%s)" % ", ".join(v.split(".") + [repr(s), '0'])


def _version_part(v, i):
    parts = v.split(".")
    if len(parts) <= i:
        return None
    return parts[i]


version_variables: dict[str, Callable[[str, Optional[str]], str]] = {
    'TUPLED_VERSION': lambda v, s: "(%s)" % ", ".join(v.split(".")),
    'STATUS_TUPLED_VERSION': _status_tupled_version,
    'VERSION': lambda v, s: v,
    'MAJOR_VERSION': lambda v, s: _version_part(v, 0),
    'MINOR_VERSION': lambda v, s: _version_part(v, 1),
    'MICRO_VERSION': lambda v, s: _version_part(v, 2),
}


def _version_line_re(new_line: str) -> re.Pattern:
    ps = []
    ver_match = '|'.join([f'\\${k}' for k in version_variables])
    for p in re.split(r'(' + ver_match + r')', new_line):
        if p and p[0] == '$' and p[1:] in version_variables:
            ps.append('(?P<' + p[1:].lower() + '>.*)')
        else:
            ps.append(re.escape(p))

    return re.compile(''.join(ps).encode())


def update_version_in_file(
        tree: MutableTree, update_cfg, new_version: str, status: str) -> None:
    with tree.get_file(update_cfg.path) as f:
        lines = list(f.readlines())
    matches = 0
    if not update_cfg.match:
        r = _version_line_re(update_cfg.new_line)
    else:
        r = re.compile(update_cfg.match.encode())
    for i, line in enumerate(lines):
        if not r.match(line):
            continue
        new_line = update_cfg.new_line.encode()
        for k, vfn in version_variables.items():
            v = vfn(new_version, status)
            if v is not None:
                new_line = new_line.replace(b"$" + k.encode(), v.encode())
            else:
                if (b'$' + k.encode()) in new_line:
                    raise ValueError(
                        f'no expansion for variable ${k} used in {new_line}')
        lines[i] = new_line + b"\n"
        matches += 1
    if matches == 0:
        raise Exception(
            f"No matches for {update_cfg.match} in {update_cfg.path}")
    tree.put_file_bytes_non_atomic(update_cfg.path, b"".join(lines))


update_version_in_manpage = _disperse_rs.update_version_in_manpage


def check_release_age(branch: Branch, cfg: ProjectConfig, now: datetime) -> None:
    rev = branch.repository.get_revision(branch.last_revision())
    if cfg.timeout_days is not None:
        commit_time = datetime.fromtimestamp(rev.timestamp)
        time_delta = now - commit_time
        if time_delta.days < cfg.timeout_days:
            raise RecentCommits(time_delta.days, cfg.timeout_days)


def reverse_version(
        update_cfg, lines: List[bytes]) -> Tuple[Optional[str], Optional[str]]:
    r = _version_line_re(update_cfg.new_line)
    for line in lines:
        m = r.match(line)
        if not m:
            continue
        try:
            return m.group('version').decode(), None
        except IndexError:
            pass
        try:
            return (
                '.'.join(map(str, eval(m.group('tupled_version').decode()))),
                None)
        except IndexError:
            pass
        try:
            val = eval(m.group('status_tupled_version').decode())
            return '.'.join(map(str, val[:-2])), val[-2]
        except IndexError:
            pass
    return None, None


def find_last_version(tree: WorkingTree, cfg) -> Tuple[str, Optional[str]]:
    if tree.has_filename("Cargo.toml"):
        logging.debug("Reading version from Cargo.toml")
        return find_version_in_cargo(tree), None
    if tree.has_filename("pyproject.toml"):
        logging.debug("Reading version from pyproject.toml")
        version = find_version_in_pyproject_toml(tree)
        if version:
            return version, None
        if pyproject_uses_hatch_vcs(tree):
            version = find_hatch_vcs_version(tree)
            if not version:
                raise NotImplementedError(
                    "hatch in use but unable to find hatch vcs version")
            return version, None
    if cfg.update_version:
        for update_cfg in cfg.update_version:
            logging.debug("Reading version from %s", update_cfg.path)
            with tree.get_file(update_cfg.path) as f:
                lines = list(f.readlines())
            v, s = reverse_version(update_cfg, lines)
            if v:
                return v, s
        raise KeyError
    else:
        raise NotImplementedError


check_new_revisions = _disperse_rs.check_new_revisions

expand_tag = _disperse_rs.expand_tag
unexpand_tag = _disperse_rs.unexpand_tag


def release_project(   # noqa: C901
        repo_url: str, *, force: bool = False,
        new_version: Optional[str] = None,
        dry_run: bool = False, ignore_ci: bool = False):
    from breezy.controldir import ControlDir
    from breezy.transport.local import LocalTransport
    try:
        from breezy.errors import ConnectionError  # type: ignore
    except ImportError:
        pass

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
        logging.info('Using public branch %s', public_repo_url)
    elif (branch.get_submit_branch()
            and not branch.get_submit_branch().startswith('file:')):
        public_repo_url = branch.get_submit_branch()
        public_branch = Branch.open(public_repo_url)
        local_branch = branch
        logging.info('Using public branch %s', public_repo_url)
    else:
        public_repo_url = branch.get_push_location()
        public_branch = Branch.open(public_repo_url)
        local_branch = branch
        logging.info('Using public branch %s', public_repo_url)

    if public_repo_url:
        public_repo_url = split_segment_parameters(public_repo_url)[0]
    else:
        public_repo_url = None

    if public_repo_url:
        logging.info('Found public repository URL: %s', public_repo_url)

    with Workspace(public_branch, resume_branch=local_branch) as ws:
        try:
            cfg = read_project_with_fallback(ws.local_tree)
        except NoSuchFile as exc:
            no_disperse_config.inc()
            raise NodisperseConfig() from exc

        name: Optional[str]
        if cfg.name:
            name = cfg.name
        elif ws.local_tree.has_filename('pyproject.toml'):
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
                check_gh_repo_action_status(
                    gh_repo, cfg.github_branch or 'HEAD')
            except (GitHubStatusFailed, GitHubStatusPending) as e:
                if ignore_ci:
                    ci_ignored_count.labels(project=name).inc()
                    logging.warning('Ignoring failing CI: %s', e)
                else:
                    raise
        else:
            possible_urls = []
            if ws.local_tree.has_filename('setup.cfg'):
                possible_urls.extend(
                    read_project_urls_from_setup_cfg(
                        ws.local_tree.abspath('setup.cfg')))
            if ws.local_tree.has_filename('pyproject.toml'):
                possible_urls.extend(
                    read_project_urls_from_pyproject_toml(
                        ws.local_tree.abspath('pyproject.toml')))
            if public_repo_url is not None:
                possible_urls.append((public_repo_url, public_branch.name))

            for url, branch_name in possible_urls:
                parsed_url = urlparse(url)
                hostname = parsed_url.hostname
                if hostname == 'github.com':
                    gh_repo = get_github_repo(url)
                    try:
                        check_gh_repo_action_status(gh_repo, branch_name)
                    except (GitHubStatusFailed, GitHubStatusPending) as e:
                        if ignore_ci:
                            logging.warning('Ignoring failing CI: %s', e)
                            ci_ignored_count.labels(project=name).inc()
                        else:
                            raise
                    break
                elif hostname == 'launchpad.net':
                    parts = parsed_url.path.strip('/').split('/')[0]
                    launchpad_project = get_launchpad_project(parts[0])
                    if len(parts) > 1 and not parts[1].startswith('+'):
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
                        ws.local_tree, cfg)
                except NotImplementedError:
                    last_version, last_version_status = find_last_version_in_tags(
                        ws.local_tree.branch, cfg.tag_name)
                last_version_tag_name = expand_tag(cfg.tag_name, last_version)
                if ws.local_tree.branch.tags.has_tag(last_version_tag_name):
                    new_version = increase_version(last_version)
                else:
                    new_version = last_version
            assert new_version
            logging.info('Picked new version: %s', new_version)

        assert " " not in str(new_version), "Invalid version %r" % new_version

        if cfg.pre_dist_command:
            try:
                subprocess.check_call(
                    cfg.pre_dist_command, cwd=ws.local_tree.abspath('.'),
                    shell=True)
            except subprocess.CalledProcessError as e:
                pre_dist_command_failed.labels(project=name).inc()
                raise PreDistCommandFailed(cfg.pre_dist_command, e.returncode)

        if cfg.verify_command:
            verify_command = cfg.verify_command
        else:
            if ws.local_tree.has_filename('tox.ini'):
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
            update_version_in_file(ws.local_tree, update_version, new_version, "final")
        for update_manpage in cfg.update_manpages:
            for path in glob(ws.local_tree.abspath(update_manpage)):
                update_version_in_manpage(
                    ws.local_tree, ws.local_tree.relpath(path), new_version,
                    now)
        if ws.local_tree.has_filename("Cargo.toml"):
            update_version_in_cargo(ws.local_tree, new_version)
        if ws.local_tree.has_filename("pyproject.toml"):
            update_version_in_pyproject_toml(ws.local_tree, new_version)
        ws.local_tree.commit(f"Release {new_version}.")

        if verify_command:
            try:
                subprocess.check_call(
                    verify_command, cwd=ws.local_tree.abspath("."),
                    shell=True
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
        logging.info('Creating tag %s', tag_name)
        if hasattr(ws.local_tree.branch.repository, '_git'):
            subprocess.check_call(
                ["git", "tag", "-as", tag_name,
                 "-m", f"Release {new_version}"],
                cwd=ws.local_tree.abspath("."),
            )
        else:
            ws.local_tree.branch.tags.set_tag(
                tag_name, ws.local_tree.last_revision())
        if ws.local_tree.has_filename("setup.py"):
            pypi_paths = create_setup_py_artifacts(ws.local_tree)
        elif ws.local_tree.has_filename("pyproject.toml"):
            pypi_paths = create_python_artifacts(ws.local_tree)
        else:
            pypi_paths = None

        artifacts = []
        if not dry_run:
            ws.push_tags(tags={
                tag_name: ws.local_tree.branch.tags.lookup_tag(tag_name)})
        try:
            # Wait for CI to go green
            if gh_repo:
                if dry_run:
                    logging.info('In dry-run mode, so unable to wait for CI')
                else:
                    wait_for_gh_actions(
                        gh_repo, tag_name,
                        timeout=(cfg.ci_timeout or DEFAULT_CI_TIMEOUT))

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
            logging.info('Deleting remote tag %s', tag_name)
            if not dry_run:
                ws.main_branch.tags.delete_tag(tag_name)
            raise

        # At this point, it's official - so let's push.
        try:
            if not dry_run:
                ws.push()
        except ProtectedBranchHookDeclined:
            branch_protected_count.labels(project=name).inc()
            logging.info('branch %s is protected; proposing merge instead',
                         ws.local_tree.branch.name)
            if not dry_run:
                (mp, _is_new) = ws.propose(
                    description=f"Merge release of {new_version}",
                    tags=[tag_name],
                    name=f'release-{new_version}', labels=['release'],
                    commit_message=f"Merge release of {new_version}")
            else:
                mp = None
            logging.info(f'Created merge proposal: {mp.url}')

            if getattr(mp, 'supports_auto_merge', False):
                mp.merge(auto=True, message=f"Merge release of {new_version}")

        if gh_repo:
            if dry_run:
                logging.info(
                    "skipping creation of github release due to dry run mode")
            else:
                create_github_release(
                    gh_repo, tag_name, new_version, release_changes)

        if launchpad_project:
            if dry_run:
                logging.info(
                    "skipping upload of tarball to Launchpad")
            else:
                lp_release = ensure_launchpad_release(
                    launchpad_project, new_version,
                    series_name=launchpad_series,
                    release_notes=release_changes)
                add_launchpad_release_files(lp_release, artifacts)

        # TODO(jelmer): Mark any news bugs in NEWS as fixed [later]
        # * Commit:
        #  * Update NEWS and version strings for next version
        new_pending_version = increase_version(new_version, -1)
        logging.info('Using new version %s', new_pending_version)
        if news_file:
            news_file.add_pending(new_pending_version)
            ws.local_tree.commit(f'Start on {new_pending_version}')
            if not dry_run:
                ws.push()
        if launchpad_project:
            if dry_run:
                logging.info(
                    'Skipping creation of new milestone %s on Launchpad',
                    new_pending_version)
            else:
                create_launchpad_milestone(
                    launchpad_project, new_pending_version,
                    series_name=launchpad_series)
    if not dry_run:
        if local_wt is not None:
            local_wt.pull(public_branch)
        elif local_branch is not None:
            local_branch.pull(public_branch)

    released_count.labels(project=name).inc()
    return name, new_version


def info_many(urls):
    from breezy.controldir import ControlDir
    try:
        from breezy.errors import ConnectionError  # type: ignore
    except ImportError:
        pass
    from breezy.errors import UnsupportedOperation


    ret = 0

    for url in urls:
        if url != ".":
            logging.info('Processing %s', url)

        try:
            local_wt, branch = ControlDir.open_tree_or_branch(url)
        except ConnectionError as e:
            ret = 1
            logging.error('Unable to connect to %s: %s', url, e)
            continue

        if local_wt is not None:
            info(local_wt)
        else:
            try:
                info(branch.basis_tree())
            except UnsupportedOperation:
                with Workspace(branch) as ws:
                    info(ws.local_tree)

    return ret


find_last_version_in_tags = _disperse_rs.find_last_version_in_tags


def info(wt):

    try:
        cfg = read_project_with_fallback(wt)
    except NoSuchFile:
        logging.info("No configuration found")
        return

    name: Optional[str]

    if cfg.name:
        name = cfg.name
    elif wt.has_filename('pyproject.toml'):
        name = find_name_in_pyproject_toml(wt)
    else:
        name = None

    logging.info("Project: %s", name)

    try:
        last_version, last_version_status = find_last_version(wt, cfg)
    except NotImplementedError:
        last_version, last_version_status = find_last_version_in_tags(wt.branch, cfg)
    logging.info("Last release: %s", last_version)
    if last_version_status:
        logging.info("  status: %s", last_version_status)

    tag_name = expand_tag(cfg.tag_name, last_version)
    try:
        release_revid = wt.branch.tags.lookup_tag(tag_name)
    except NoSuchTag:
        logging.info("  tag %s for previous release not found", tag_name)
    else:
        logging.info("  tag name: %s (%s)", tag_name, release_revid.decode('utf-8'))

        rev = wt.branch.repository.get_revision(release_revid)
        logging.info("  date: %s", datetime.fromtimestamp(rev.timestamp))

        if rev.revision_id != wt.branch.last_revision():
            graph = wt.branch.repository.get_graph()
            missing = list(
                graph.iter_lefthand_ancestry(
                    wt.branch.last_revision(), [release_revid]))
            if missing[-1] == NULL_REVISION:
                logging.info("  last release not found in ancestry")
            else:
                first = wt.branch.repository.get_revision(missing[-1])
                first_age = datetime.now() - datetime.fromtimestamp(first.timestamp)
                logging.info("  %d revisions since last release. First is %d days old.",
                             len(missing), first_age.days)
        else:
            logging.info("  no revisions since last release")

    try:
        new_version = find_pending_version(wt, cfg)
    except NotImplementedError:
        logging.info("No pending version found; would use %s",
                     increase_version(last_version, -1))
    except NoUnreleasedChanges:
        logging.info("No unreleased changes")
    except OddPendingVersion as e:
        logging.info("Pending version: %s (odd)", e.version)
    else:
        logging.info("Pending version: %s", new_version)


def validate_config(path):
    wt = WorkingTree.open(path)

    try:
        cfg = read_project_with_fallback(wt)
    except NoSuchFile as exc:
        raise NodisperseConfig() from exc

    if cfg.news_file:
        news_file = NewsFile(wt, cfg.news_file)
        news_file.validate()

    if cfg.update_version:
        for update_cfg in cfg.update_version:
            if not update_cfg.match:
                r = _version_line_re(update_cfg.new_line)
            else:
                r = re.compile(update_cfg.match.encode())
            with wt.get_file(update_cfg.path) as f:
                for line in f:
                    if r.match(line):
                        break
                else:
                    raise Exception(
                        f"No matches for {r.pattern} in {update_cfg.path}")

    for update in cfg.update_manpages:
        if not glob(wt.abspath(update)):
            raise Exception("no matches for {update}")


def release_many(urls, *, force=False, dry_run=False, discover=False,  # noqa: C901
                 new_version=None, ignore_ci=False):
    failed: List[Tuple[str, Exception]] = []
    skipped: List[Tuple[str, Exception]] = []
    success = []
    ret = 0
    for url in urls:
        if url != ".":
            logging.info('Processing %s', url)
        try:
            release_project(
                url, force=force, new_version=new_version,
                dry_run=dry_run, ignore_ci=ignore_ci)
        except RecentCommits as e:
            logging.info(
                "Recent commits exist (%d < %d)", e.min_commit_age,
                e.commit_age)
            skipped.append((url, e))
            if not discover:
                ret = 1
        except VerifyCommandFailed as e:
            logging.error('Verify command (%s) failed to run.', e.command)
            failed.append((url, e))
            ret = 1
        except PreDistCommandFailed as e:
            logging.error('Pre-Dist command (%s) failed to run.', e.command)
            failed.append((url, e))
            ret = 1
        except UploadCommandFailed as e:
            logging.error('Upload command (%s) failed to run.', e.args[0])
            failed.append((url, e))
            ret = 1
        except ReleaseTagExists as e:
            logging.warning(
                '%s: Release tag %s for version %s exists. '
                'Unmerged release commit?',
                e.project, e.tag_name, e.version)
            skipped.append((url, e))
            if not discover:
                ret = 1
        except DistCreationFailed as e:
            logging.error('Dist creation failed to run: %s', e)
            failed.append((url, e))
            ret = 1
        except NoUnreleasedChanges as e:
            logging.error('No unreleased changes')
            skipped.append((url, e))
            if not discover:
                ret = 1
        except NodisperseConfig as e:
            logging.error('No configuration for disperse')
            skipped.append((url, e))
            if not discover:
                ret = 1
        except GitHubStatusPending as e:
            logging.error(
                'GitHub checks for commit %s '
                'not finished yet. See %s', e.sha, e.html_url)
            failed.append((url, e))
            ret = 1
        except GitHubStatusFailed as e:
            logging.error(
                'GitHub check for commit %s failed. '
                'See %s', e.sha, e.html_url)
            failed.append((url, e))
            ret = 1
        except RepositoryUnavailable as e:
            logging.error('Repository is unavailable: %s', e.args[0])
            failed.append((url, e))
            ret = 1
        except OddPendingVersion as e:
            logging.error('Odd pending version: %s', e.version)
            failed.append((url, e))
            ret = 1
        else:
            success.append(url)

    if discover:
        logging.info('%s successfully released, %s skipped, %s failed',
                     len(success), len(skipped), len(failed))
    return ret
