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
from typing import Optional, List
from urllib.request import urlopen
from urllib.parse import urlparse

from github import Github  # type: ignore

from breezy.urlutils import split_segment_parameters
import breezy.git  # noqa: F401
import breezy.bzr  # noqa: F401
from breezy.errors import NoSuchFile
try:
    from breezy.plugins.github.forge import retrieve_github_token
except ModuleNotFoundError:
    from breezy.plugins.github.hoster import retrieve_github_token
from breezy.git.remote import RemoteGitError
from breezy.branch import Branch
from breezy.tree import InterTree, Tree
from breezy.revision import NULL_REVISION
from silver_platter.workspace import Workspace


from . import NoUnreleasedChanges
from .news_file import (
    NewsFile,
    news_find_pending,
    )


class RecentCommits(Exception):
    """Indicates there are too recent commits for a package."""

    def __init__(self, commit_age, min_commit_age):
        self.commit_age = commit_age
        self.min_commit_age = min_commit_age
        super(RecentCommits, self).__init__(
            "Last commit is only %d days old (< %d)"
            % (self.commit_age, self.min_commit_age)
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


class NoReleaserConfig(Exception):
    """No releaser config present"""


def increase_version(version: str, idx: int = -1) -> str:
    parts = [int(x) for x in version.split('.')]
    parts[idx] += 1
    return '.'.join(map(str, parts))


def find_pending_version(tree: Tree, cfg) -> str:
    if cfg.news_file:
        return news_find_pending(tree, cfg.news_file)
    else:
        raise NotImplementedError


def _version_line_re(new_line: str) -> re.Match:
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
            new_version.split(".") + [status, '0'])
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
            "No matches for %s in %s" % (update_cfg.match, update_cfg.path))
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
            raise Exception('Unable to find format for date %s' % args[3])
        for r, f in VERSION_OPTIONS:
            m = re.fullmatch(r, args[4])
            if m:
                args[4] = re.sub(
                    r, f.replace('$VERSION', new_version), args[4])
                break
        lines[i] = shlex.join(args).encode() + b'\n'
        break
    else:
        raise Exception("No matches for date or version in %s" % (path, ))
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


def reverse_version(update_cfg, lines: List[bytes]) -> Optional[str]:
    r = _version_line_re(update_cfg.new_line)
    for line in lines:
        m = r.match(line)
        if not m:
            continue
        try:
            return m.group('version').decode()
        except IndexError:
            pass
        try:
            return '.'.join(map(str, eval(m.group('tupled_version').decode())))
        except IndexError:
            pass
        try:
            return '.'.join(
                map(str, eval(m.group('status_tupled_version').decode())))
        except IndexError:
            pass
    return None


def find_last_version(tree: Tree, cfg) -> str:
    if cfg.update_version:
        for update_cfg in cfg.update_version:
            with tree.get_file(update_cfg.path) as f:
                lines = list(f.readlines())
            v = reverse_version(update_cfg, lines)
            if v:
                return v
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


def release_project(   # noqa: C901
        repo_url: str, force: bool = False,
        new_version: Optional[str] = None,
        dry_run: bool = False):
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
            with ws.local_tree.get_file("releaser.conf") as f:
                cfg = read_project(f)
        except NoSuchFile:
            raise NoReleaserConfig()

        if cfg.github_url:
            gh_repo = get_github_repo(cfg.github_url)
            check_gh_repo_action_status(gh_repo, cfg.github_branch or 'HEAD')
        elif (public_repo_url is not None and
              urlparse(public_repo_url).hostname == 'github.com'):
            gh_repo = get_github_repo(public_repo_url)
            check_gh_repo_action_status(gh_repo, public_branch.name)
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
                last_version = find_last_version(ws.local_tree, cfg)
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
        ws.local_tree.commit("Release %s." % new_version)
        tag_name = cfg.tag_name.replace("$VERSION", new_version)
        logging.info('Creating tag %s', tag_name)
        if hasattr(ws.local_tree.branch.repository, '_git'):
            subprocess.check_call(
                ["git", "tag", "-as", tag_name,
                 "-m", "Release %s" % new_version],
                cwd=ws.local_tree.abspath("."),
            )
        else:
            ws.local_tree.branch.tags.set_tag(
                tag_name, ws.local_tree.last_revision())
        if ws.local_tree.has_filename("setup.py"):
            try:
                subprocess.check_call(
                    ["./setup.py", "sdist"], cwd=ws.local_tree.abspath(".")
                )
            except subprocess.CalledProcessError as e:
                raise DistCommandFailed("setup.py sdist", e.returncode)
            from distutils.core import run_setup

            orig_dir = os.getcwd()
            try:
                os.chdir(ws.local_tree.abspath('.'))
                result = run_setup(
                    ws.local_tree.abspath("setup.py"), stop_after="config")
            finally:
                os.chdir(orig_dir)
            pypi_path = os.path.join(
                "dist", "%s-%s.tar.gz" % (
                    result.get_name(), new_version)  # type: ignore
            )
            command = [
                "twine", "upload", "--non-interactive", "--sign", pypi_path]
            if dry_run:
                logging.info("skipping twine upload due to dry run mode")
            elif cfg.skip_twine_upload:
                logging.info("skipping twine upload; disabled in config")
            else:
                try:
                    subprocess.check_call(
                        command, cwd=ws.local_tree.abspath("."))
                except subprocess.CalledProcessError as e:
                    raise UploadCommandFailed(command, e.returncode)
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
                subprocess.check_call(
                    ["scp", ws.local_tree.abspath(pypi_path), loc])
        # At this point, it's official - so let's push.
        try:
            ws.push(tags=[tag_name], dry_run=dry_run)
        except RemoteGitError as e:
            if str(e) == "protected branch hook declined":
                logging.info('branch %s is protected; proposing merge instead',
                             ws.local_tree.branch.name)
                (mp, is_new) = ws.propose(
                    description="Merge release of %s" % new_version,
                    tags=[tag_name],
                    name='release-%s' % new_version, labels=['release'],
                    dry_run=dry_run,
                    commit_message="Merge release of %s" % new_version)
                logging.info('Created merge proposal: %s', mp.url)
            else:
                raise

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
            ws.local_tree.commit('Start on %s' % new_pending_version)
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
    token = retrieve_github_token(parsed_url.scheme, parsed_url.hostname)
    gh = Github(token)
    logging.info('Finding project %s on GitHub', fullname)
    return gh.get_repo(fullname)


class GitHubCheckRunFailed(Exception):
    """A check run failed."""

    def __init__(self, name, conclusion, sha, branch, html_url):
        self.name = name
        self.conclusion = conclusion
        self.sha = sha
        self.branch = branch
        self.html_url = html_url


def check_gh_repo_action_status(repo, branch):
    if not branch:
        branch = 'HEAD'
    commit = repo.get_commit(branch)
    for check_run in commit.get_check_runs():
        if check_run.conclusion not in ('success', 'skipped'):
            raise GitHubCheckRunFailed(
                check_run.name, check_run.conclusion, commit.sha, branch,
                check_run.html_url)


def create_github_release(repo, tag_name, version, description):
    logging.info('Creating release on GitHub')
    repo.create_git_release(
        tag=tag_name, name=version, draft=False, prerelease=False,
        message=description or ('Release %s.' % version))


def pypi_discover_urls():
    import xmlrpc.client
    from configparser import RawConfigParser
    client = xmlrpc.client.ServerProxy('https://pypi.org/pypi')
    pypi_username = os.environ.get('PYPI_USERNAME')
    if pypi_username is None:
        cp = RawConfigParser()
        config_file_path = os.environ.get(
            'TWINE_CONFIG_FILE', os.path.expanduser('~/.pypirc'))
        cp.read(config_file_path)
        pypi_username = cp.get('pypi', 'username')
    if pypi_username == '__token__':
        logging.warning('Unable to determine pypi username')
        return []
    ret = []
    for relation, package in client.user_packages(pypi_username):
        with urlopen('https://pypi.org/pypi/%s/json' % package) as f:
            data = json.load(f)
        project_urls = data['info']['project_urls']
        if project_urls is None:
            logging.warning('Project %s does not have project URLs', package)
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


def main(argv=None):  # noqa: C901
    import argparse

    parser = argparse.ArgumentParser("releaser")
    parser.add_argument("url", nargs="?", type=str)
    parser.add_argument(
        "--new-version", type=str, help='New version to release.')
    parser.add_argument(
        "--discover", action='store_true',
        help='Discover relevant projects to release')
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Dry run, don't actually create a release.")
    parser.add_argument(
        "--force", action="store_true",
        help='Force a new release, even if timeout is not reached.')
    parser.add_argument(
        "--try", action="store_true",
        help="Do not exit with non-zero if projects failed to be released.")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO, format='%(message)s')

    if not args.discover:
        urls = [args.url or "."]
    else:
        if args.new_version:
            parser.print_usage()
            return 1
        urls = pypi_discover_urls()

    failed = []
    skipped = []
    success = []
    ret = 0
    for url in urls:
        if url != ".":
            logging.info('Processing %s', url)
        try:
            release_project(
                url, force=args.force, new_version=args.new_version,
                dry_run=args.dry_run)
        except RecentCommits as e:
            logging.error(
                "Recent commits exist (%d < %d)", e.min_commit_age,
                e.commit_age)
            skipped.append((url, e))
            if not args.discover:
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
            if not args.discover:
                ret = 1
        except NoReleaserConfig as e:
            logging.error('No configuration for releaser')
            skipped.append((url, e))
            if not args.discover:
                ret = 1
        except GitHubCheckRunFailed as e:
            if e.conclusion is None:
                logging.error(
                    'GitHub check %s for commit %s (branch: %s) '
                    'not finished yet. See %s', e.name, e.sha, e.branch,
                    e.html_url)
            else:
                logging.error(
                    'GitHub check %s (%s) for commit %s (branch: %s) failed. '
                    'See %s', e.name, e.conclusion, e.sha, e.branch,
                    e.html_url)
            failed.append((url, e))
            ret = 1
        else:
            success.append(url)

    if args.discover:
        logging.info('%s successfully released, %s skipped, %s failed',
                     len(success), len(skipped), len(failed))
    if getattr(args, 'try', False):
        return 0
    return ret


if __name__ == "__main__":
    sys.exit(main())
