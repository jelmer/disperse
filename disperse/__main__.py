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

from datetime import datetime
from glob import glob
import json
import logging
import os
import re
import subprocess
import sys
import time
from typing import Optional, List, Tuple
from urllib.request import urlopen, Request
from urllib.parse import urlparse

from github import Github  # type: ignore

from breezy.urlutils import split_segment_parameters
import breezy.git  # noqa: F401
import breezy.bzr  # noqa: F401
from breezy.transport import NoSuchFile
from breezy.plugins.github.forge import retrieve_github_token
from breezy.git.remote import ProtectedBranchHookDeclined
from breezy.branch import Branch
from breezy.tree import InterTree, Tree
from breezy.revision import NULL_REVISION
from silver_platter.workspace import Workspace


from . import NoUnreleasedChanges, version_string
from .news_file import (
    NewsFile,
    news_find_pending,
    )


DEFAULT_CI_TIMEOUT = 7200


class RecentCommits(Exception):
    """Indicates there are too recent commits for a package."""

    def __init__(self, commit_age, min_commit_age):
        self.commit_age = commit_age
        self.min_commit_age = min_commit_age
        super(RecentCommits, self).__init__(
            f"Last commit is only {self.commit_age} days old "
            f"(< {self.min_commit_age})"
        )


class VerifyCommandFailed(Exception):

    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


class UploadCommandFailed(Exception):

    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


class DistCommandFailed(Exception):

    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


class NodisperseConfig(Exception):
    """No disperse config present"""


def increase_version(version: str, idx: int = -1) -> str:
    parts = [int(x) for x in version.split('.')]
    parts[idx] += 1
    return '.'.join(map(str, parts))


def find_pending_version(tree: Tree, cfg) -> str:
    if cfg.news_file:
        return news_find_pending(tree, cfg.news_file)
    else:
        raise NotImplementedError


def _version_line_re(new_line: str) -> re.Pattern:
    ps = []
    for p in re.split(
            r'(\$TUPLED_VERSION|\$VERSION|\$STATUS_TUPLED_VERSION)',
            new_line):
        if p in ('$TUPLED_VERSION', '$VERSION', '$STATUS_TUPLED_VERSION'):
            ps.append('(?P<' + p[1:].lower() + '>.*)')
        else:
            ps.append(re.escape(p))

    return re.compile(''.join(ps).encode())


def update_version_in_file(
        tree: Tree, update_cfg, new_version: str, status: str) -> None:
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
        tupled_version = "(%s)" % ", ".join(new_version.split("."))
        status_tupled_version = "(%s)" % ", ".join(
            new_version.split(".") + [repr(status), '0'])
        lines[i] = (
            update_cfg.new_line.encode()
            .replace(b"$VERSION", new_version.encode())
            .replace(b"$TUPLED_VERSION", tupled_version.encode())
            .replace(b"$STATUS_TUPLED_VERSION", status_tupled_version.encode())
            + b"\n"
        )
        matches += 1
    if matches == 0:
        raise Exception(
            f"No matches for {update_cfg.match} in {update_cfg.path}")
    tree.put_file_bytes_non_atomic(update_cfg.path, b"".join(lines))


def update_version_in_manpage(
        tree: Tree, path, new_version: str, release_date: datetime) -> None:
    with tree.get_file(path) as f:
        lines = list(f.readlines())
    DATE_OPTIONS = [
        ('20[0-9][0-9]-[0-1][0-9]-[0-3][0-9]', "%Y-%m-%d"),
        ('[A-Za-z]+ ([0-9]{4})', '%B %Y'),
    ]
    VERSION_OPTIONS = [
        ('([^ ]+) ([0-9a-z.]+)', r'\1 $VERSION'),
    ]
    import shlex
    for i, line in enumerate(lines):
        if not line.startswith(b'.TH '):
            continue
        args = shlex.split(line.decode())
        for r, f in DATE_OPTIONS:
            m = re.fullmatch(r, args[3])
            if m:
                args[3] = release_date.strftime(f)
                break
        else:
            raise Exception(f'Unable to find format for date {args[3]}')
        for r, f in VERSION_OPTIONS:
            m = re.fullmatch(r, args[4])
            if m:
                args[4] = re.sub(
                    r, f.replace('$VERSION', new_version), args[4])
                break
        lines[i] = shlex.join(args).encode() + b'\n'
        break
    else:
        raise Exception(f"No matches for date or version in {path}")
    tree.put_file_bytes_non_atomic(path, b"".join(lines))


def update_version_in_cargo(tree: Tree, new_version: str) -> None:
    from toml.decoder import load, TomlPreserveCommentDecoder
    from toml.encoder import dumps, TomlPreserveCommentEncoder
    with open(tree.abspath('Cargo.toml'), 'r') as f:
        d = load(f, dict, TomlPreserveCommentDecoder())
    d['package']['version'] = new_version
    tree.put_file_bytes_non_atomic(
        'Cargo.toml',
        dumps(d, TomlPreserveCommentEncoder()).encode())  # type: ignore
    subprocess.check_call(['cargo', 'update'], cwd=tree.abspath('.'))


def check_release_age(branch: Branch, cfg, now: datetime) -> None:
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


def find_last_version(tree: Tree, cfg) -> Tuple[str, Optional[str]]:
    if cfg.update_version:
        for update_cfg in cfg.update_version:
            with tree.get_file(update_cfg.path) as f:
                lines = list(f.readlines())
            v, s = reverse_version(update_cfg, lines)
            if v:
                return v, s
        raise KeyError
    else:
        raise NotImplementedError


def check_new_revisions(
        branch: Branch, news_file_path: Optional[str] = None) -> None:
    tags = branch.tags.get_reverse_tag_dict()
    graph = branch.repository.get_graph()
    from_tree = None
    with branch.lock_read():
        for revid in graph.iter_lefthand_ancestry(branch.last_revision()):
            if tags.get(revid):
                from_tree = branch.repository.revision_tree(revid)
                break
        else:
            from_tree = branch.repository.revision_tree(NULL_REVISION)
        last_tree = branch.basis_tree()
        delta = InterTree.get(from_tree, last_tree).compare()
        if news_file_path:
            for i in range(len(delta.modified)):
                if delta.modified[i].path == (news_file_path, news_file_path):
                    del delta.modified[i]
                    break
        if not delta.has_changed():
            raise NoUnreleasedChanges()


def upload_python_artifacts(local_tree, pypi_paths):
    command = [
        "twine", "upload", "--non-interactive",
        "--sign"] + pypi_paths
    try:
        subprocess.check_call(command, cwd=local_tree.abspath("."))
    except subprocess.CalledProcessError as e:
        raise UploadCommandFailed(command, e.returncode)


def create_python_artifacts(local_tree):
    # Import setuptools, just in case it tries to replace distutils
    import setuptools  # noqa: F401
    from distutils.core import run_setup

    orig_dir = os.getcwd()
    try:
        os.chdir(local_tree.abspath('.'))
        result = run_setup(
            local_tree.abspath("setup.py"), stop_after="config")
    finally:
        os.chdir(orig_dir)
    pypi_paths = []
    is_pure = (
        not result.has_c_libraries()  # type: ignore
        and not result.has_ext_modules())  # type: ignore
    if is_pure:
        try:
            subprocess.check_call(
                ["./setup.py", "bdist_wheel"],
                cwd=local_tree.abspath(".")
            )
        except subprocess.CalledProcessError as e:
            raise DistCommandFailed(
                "setup.py bdist_wheel", e.returncode)
        wheels_glob = 'dist/%s-%s-*-any.whl' % (
            result.get_name().replace('-', '_'),  # type: ignore
            result.get_version())  # type: ignore
        wheels = glob(
            os.path.join(local_tree.abspath('.'), wheels_glob))
        if not wheels:
            raise AssertionError(
                'setup.py bdist_wheel did not produce expected files. '
                'glob: %r, files: %r' % (
                    wheels_glob,
                    os.listdir(local_tree.abspath('dist'))))
        pypi_paths.extend(wheels)
    else:
        logging.warning(
            'python module is not pure; not uploading binary wheels')
    try:
        subprocess.check_call(
            ["./setup.py", "sdist"], cwd=local_tree.abspath(".")
        )
    except subprocess.CalledProcessError as e:
        raise DistCommandFailed("setup.py sdist", e.returncode)
    sdist_path = os.path.join(
        "dist", "%s-%s.tar.gz" % (
            result.get_name(), result.get_version()))  # type: ignore
    pypi_paths.append(sdist_path)
    return pypi_paths


def release_project(   # noqa: C901
        repo_url: str, *, force: bool = False,
        new_version: Optional[str] = None,
        dry_run: bool = False, ignore_ci: bool = False):
    from .config import read_project
    from breezy.controldir import ControlDir
    from breezy.transport.local import LocalTransport

    now = datetime.now()
    local_wt, branch = ControlDir.open_tree_or_branch(repo_url)

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
    elif (branch.get_submit_branch() and
            not branch.get_submit_branch().startswith('file:')):
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
            with ws.local_tree.get_file("disperse.conf") as f:
                cfg = read_project(f)
        except NoSuchFile as exc:
            try:
                with ws.local_tree.get_file("releaser.conf") as f:
                    cfg = read_project(f)
            except NoSuchFile:
                raise NodisperseConfig() from exc

        if cfg.github_url:
            gh_repo = get_github_repo(cfg.github_url)
            try:
                check_gh_repo_action_status(
                    gh_repo, cfg.github_branch or 'HEAD')
            except (GitHubStatusFailed, GitHubStatusPending) as e:
                if ignore_ci:
                    logging.warning('Ignoring failing CI: %s', e)
                else:
                    raise
        else:
            possible_urls = []
            if ws.local_tree.has_filename('setup.cfg'):
                import setuptools.config.setupcfg
                config = setuptools.config.setupcfg.read_configuration(
                    ws.local_tree.abspath('setup.cfg'))
                metadata = config.get('metadata', {})
                project_urls = metadata.get('project_urls', {})
                for key in ['GitHub', 'Source Code', 'Repository']:
                    try:
                        possible_urls.append(
                            (project_urls[key], cfg.github_branch or 'HEAD'))
                    except KeyError:
                        pass
            if public_repo_url is not None:
                possible_urls.append((public_repo_url, public_branch.name))

            for url, branch_name in possible_urls:
                if urlparse(url).hostname == 'github.com':
                    gh_repo = get_github_repo(url)
                    try:
                        check_gh_repo_action_status(gh_repo, branch_name)
                    except (GitHubStatusFailed, GitHubStatusPending) as e:
                        if ignore_ci:
                            logging.warning('Ignoring failing CI: %s', e)
                        else:
                            raise
                    break
            else:
                gh_repo = None

        check_new_revisions(ws.local_tree.branch, cfg.news_file)
        try:
            check_release_age(ws.local_tree.branch, cfg, now)
        except RecentCommits:
            if not force:
                raise
        if new_version is None:
            try:
                new_version = find_pending_version(ws.local_tree, cfg)
            except NotImplementedError:
                last_version, last_version_status = find_last_version(
                    ws.local_tree, cfg)
                last_version_tag_name = cfg.tag_name.replace(
                    "$VERSION", last_version)
                if ws.local_tree.branch.tags.has_tag(last_version_tag_name):
                    new_version = increase_version(last_version)
                else:
                    new_version = last_version
            logging.info('Using new version: %s', new_version)

        assert " " not in str(new_version), "Invalid version %r" % new_version

        if cfg.pre_dist_command:
            subprocess.check_call(
                cfg.pre_dist_command, cwd=ws.local_tree.abspath('.'),
                shell=True)

        if cfg.verify_command:
            try:
                subprocess.check_call(
                    cfg.verify_command, cwd=ws.local_tree.abspath("."),
                    shell=True
                )
            except subprocess.CalledProcessError as e:
                raise VerifyCommandFailed(cfg.verify_command, e.returncode)

        logging.info("releasing %s", new_version)
        news_file: Optional[NewsFile]
        if cfg.news_file:
            news_file = NewsFile(ws.local_tree, cfg.news_file)
            release_changes = news_file.mark_released(new_version, now)
        else:
            news_file = None
            release_changes = None
        for update in cfg.update_version:
            update_version_in_file(ws.local_tree, update, new_version, "final")
        for update in cfg.update_manpages:
            for path in glob(ws.local_tree.abspath(update)):
                update_version_in_manpage(
                    ws.local_tree, ws.local_tree.relpath(path), new_version,
                    now)
        if ws.local_tree.has_filename("Cargo.toml"):
            update_version_in_cargo(ws.local_tree, new_version)
        ws.local_tree.commit(f"Release {new_version}.")
        tag_name = cfg.tag_name.replace("$VERSION", new_version)
        assert not ws.main_branch.tags.has_tag(tag_name)
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
            pypi_paths = create_python_artifacts(ws.local_tree)
        else:
            pypi_paths = None

        artifacts = []
        ws.push_tags(tags=[tag_name], dry_run=dry_run)
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
                    subprocess.check_call(
                        ["cargo", "upload"], cwd=ws.local_tree.abspath("."))
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
            ws.push(dry_run=dry_run)
        except ProtectedBranchHookDeclined:
            logging.info('branch %s is protected; proposing merge instead',
                         ws.local_tree.branch.name)
            (mp, is_new) = ws.propose(
                description=f"Merge release of {new_version}",
                tags=[tag_name],
                name=f'release-{new_version}', labels=['release'],
                dry_run=dry_run,
                commit_message=f"Merge release of {new_version}")
            logging.info(f'Created merge proposal: {mp.url}')

        if gh_repo:
            if dry_run:
                logging.info(
                    "skipping creation of github release due to dry run mode")
            else:
                create_github_release(
                    gh_repo, tag_name, new_version, release_changes)
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
    if not dry_run:
        if local_wt is not None:
            local_wt.pull(public_branch)
        elif local_branch is not None:
            local_branch.pull(public_branch)


def get_github_repo(repo_url: str):
    if repo_url.endswith('.git'):
        repo_url = repo_url[:-4]
    parsed_url = urlparse(repo_url)
    fullname = '/'.join(parsed_url.path.strip('/').split('/')[:2])
    try:
        token = retrieve_github_token(parsed_url.scheme, parsed_url.hostname)
    except TypeError:
        # Newer versions of retrieve_github_token don't take any arguments
        token = retrieve_github_token()
    gh = Github(token)
    logging.info('Finding project %s on GitHub', fullname)
    return gh.get_repo(fullname)


class GitHubStatusFailed(Exception):

    def __init__(self, sha, url):
        self.sha = sha
        self.html_url = url


class GitHubStatusPending(Exception):

    def __init__(self, sha, url):
        self.sha = sha
        self.html_url = url


def check_gh_repo_action_status(repo, committish):
    if not committish:
        committish = 'HEAD'
    commit = repo.get_commit(committish)
    combined_status = commit.get_combined_status()
    if combined_status.state == "success":
        return
    elif combined_status.state == "pending":
        raise GitHubStatusPending(combined_status.sha, combined_status.url)
    elif combined_status.state == "failure":
        raise GitHubStatusFailed(combined_status.sha, combined_status.url)
    else:
        raise AssertionError(
            'unexpected state %s' % combined_status.state)


def wait_for_gh_actions(repo, committish, *, timeout=DEFAULT_CI_TIMEOUT):
    logging.info('Waiting for CI for %s on %s to go green', repo, committish)
    start_time = time.time()
    while time.time() - start_time < timeout:
        try:
            check_gh_repo_action_status(repo, committish)
        except GitHubStatusPending:
            time.sleep(30)
        else:
            return
    raise TimeoutError(
        'timed out waiting for CI after %d seconds' % (
            time.time() - start_time))


def create_github_release(repo, tag_name, version, description):
    logging.info('Creating release on GitHub')
    repo.create_git_release(
        tag=tag_name, name=version, draft=False, prerelease=False,
        message=description or (f'Release {version}.'))


def pypi_discover_urls(pypi_user):
    import xmlrpc.client
    client = xmlrpc.client.ServerProxy('https://pypi.org/pypi')
    ret = []
    for relation, package in client.user_packages(pypi_user):
        req = Request(
            f'https://pypi.org/pypi/{package}/json',
            headers={'Content-Type': f'disperse/{version_string}'})
        with urlopen(req) as f:
            data = json.load(f)
        project_urls = data['info']['project_urls']
        if project_urls is None:
            logging.warning(f'Project {package} does not have project URLs')
            continue
        for key, url in project_urls.items():
            if key == 'Repository':
                ret.append(url)
                break
            parsed_url = urlparse(url)
            if (parsed_url.hostname == 'github.com' and
                    parsed_url.path.strip('/').count('/') == 1):
                ret.append(url)
                break
    return ret


def validate_config(path):
    from breezy.workingtree import WorkingTree
    wt = WorkingTree.open(path)

    from .config import read_project
    try:
        with wt.get_file("disperse.conf") as f:
            cfg = read_project(f)
    except NoSuchFile as exc:
        try:
            with wt.get_file("releaser.conf") as f:
                cfg = read_project(f)
        except NoSuchFile:
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


def release_many(urls, *, force=False, dry_run=False, discover=False,
                 new_version=None, ignore_ci=False):
    failed = []
    skipped = []
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
            logging.error(
                "Recent commits exist (%d < %d)", e.min_commit_age,
                e.commit_age)
            skipped.append((url, e))
            if not discover:
                ret = 1
        except VerifyCommandFailed as e:
            logging.error('Verify command (%s) failed to run.', e.command)
            failed.append((url, e))
            ret = 1
        except UploadCommandFailed as e:
            logging.error('Upload command (%s) failed to run.', e.command)
            failed.append((url, e))
            ret = 1
        except DistCommandFailed as e:
            logging.error('Dist command (%s) failed to run.', e.command)
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
                'GitHub check for commit %failed. '
                'See %s', e.sha, e.html_url)
            failed.append((url, e))
            ret = 1
        else:
            success.append(url)

    if discover:
        logging.info('%s successfully released, %s skipped, %s failed',
                     len(success), len(skipped), len(failed))
    return ret


def main(argv=None):  # noqa: C901
    import argparse

    parser = argparse.ArgumentParser("disperse")
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Dry run, don't actually create a release.")
    subparsers = parser.add_subparsers(dest="command")
    release_parser = subparsers.add_parser("release")
    release_parser.add_argument("url", nargs="*", type=str)
    release_parser.add_argument(
        "--new-version", type=str, help='New version to release.')
    release_parser.add_argument(
        "--ignore-ci", action="store_true",
        help='Release, even if the CI is not passing.')
    discover_parser = subparsers.add_parser("discover")
    discover_parser.add_argument(
        "--pypi-user", type=str, action="append",
        help="Pypi users to upload for",
        default=os.environ.get('PYPI_USERNAME', '').split(','))
    discover_parser.add_argument(
        "--force", action="store_true",
        help='Force a new release, even if timeout is not reached.')
    discover_parser.add_argument(
        "--try", action="store_true",
        help="Do not exit with non-zero if projects failed to be released.")
    validate_parser = subparsers.add_parser("validate")
    validate_parser.add_argument("path", type=str, nargs="?", default=".")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format='%(message)s')

    if args.command == "release":
        if args.url:
            urls = args.url
        else:
            urls = ["."]
        return release_many(urls, force=True, dry_run=args.dry_run,
                            discover=False, new_version=args.new_version,
                            ignore_ci=args.ignore_ci)
    elif args.command == "discover":
        pypi_username = os.environ.get('PYPI_USERNAME')
        urls = []
        for pypi_username in args.pypi_user:
            urls.extend(pypi_discover_urls(pypi_username))
        ret = release_many(urls, force=args.force, dry_run=args.dry_run,
                           discover=True)
        if getattr(args, 'try'):
            return 0
        return ret
    elif args.command == "validate":
        return validate_config(args.path)
    else:
        parser.print_usage()


if __name__ == "__main__":
    sys.exit(main())
