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
from typing import Optional
from urllib.request import urlopen
from urllib.parse import urlparse

from github import Github  # type: ignore

import breezy.git
import breezy.bzr  # noqa: F401
from breezy.errors import NoSuchFile
from breezy.plugins.github.hoster import retrieve_github_token
from breezy.branch import Branch
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


class NoReleaserConfig(Exception):
    """No releaser config present"""


def increase_version(version, idx=-1):
    parts = [int(x) for x in version.split('.')]
    parts[idx] += 1
    return '.'.join(map(str, parts))



def find_pending_version(tree, cfg):
    if cfg.news_file:
        return news_find_pending(tree, cfg.news_file)
    else:
        raise NotImplementedError


def update_version_in_file(tree, update_cfg, new_version):
    with tree.get_file(update_cfg.path) as f:
        lines = list(f.readlines())
    matches = 0
    r = re.compile(update_cfg.match.encode())
    for i, line in enumerate(lines):
        if not r.match(line):
            continue
        tupled_version = "(%s)" % ", ".join(new_version.split("."))
        lines[i] = (
            update_cfg.new_line.encode()
            .replace(b"$VERSION", new_version.encode())
            .replace(b"$TUPLED_VERSION", tupled_version.encode())
            + b"\n"
        )
        matches += 1
    if matches == 0:
        raise Exception("No matches for %s in %s" % (update_cfg.match, update_cfg.path))
    tree.put_file_bytes_non_atomic(update_cfg.path, b"".join(lines))


def update_version_in_manpage(tree, path, new_version, release_date):
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
                args[4] = re.sub(r, f.replace('$VERSION', new_version), args[4])
                break
        lines[i] = shlex.join(args).encode() + b'\n'
        break
    else:
        raise Exception("No matches for date or version in %s" % (path, ))
    tree.put_file_bytes_non_atomic(path, b"".join(lines))


def update_version_in_cargo(tree, new_version):
    from toml.decoder import load, TomlPreserveCommentDecoder
    from toml.encoder import dumps, TomlPreserveCommentEncoder
    with open(tree.abspath('Cargo.toml'), 'r') as f:
        d = load(f, dict, TomlPreserveCommentDecoder())
    d['package']['version'] = new_version
    tree.put_file_bytes_non_atomic(
        'Cargo.toml',
        dumps(d, TomlPreserveCommentEncoder()).encode())
    subprocess.check_call(['cargo', 'update'], cwd=tree.abspath('.'))


def check_release_age(branch, cfg, now):
    rev = branch.repository.get_revision(branch.last_revision())
    if cfg.timeout_days is not None:
        commit_time = datetime.fromtimestamp(rev.timestamp)
        time_delta = now - commit_time
        if time_delta.days < cfg.timeout_days:
            raise RecentCommits(time_delta.days, cfg.timeout_days)


def find_last_version(tree, cfg):
    if cfg.update_version:
        for update_cfg in cfg.update_version:
            with tree.get_file(update_cfg.path) as f:
                lines = list(f.readlines())
            r = re.compile(update_cfg.match.encode())
            for i, line in enumerate(lines):
                m = r.match(line)
                if m:
                    try:
                        return m.group(1).decode()
                    except IndexError:
                        # No groups specified :(
                        break
        raise KeyError
    else:
        raise NotImplementedError


def release_project(   # noqa: C901
        repo_url: str, force: bool = False,
        new_version: Optional[str] = None):
    from .config import read_project

    now = datetime.now()
    branch = Branch.open(repo_url)
    with Workspace(branch) as ws:
        try:
            with ws.local_tree.get_file("releaser.conf") as f:
                cfg = read_project(f)
        except NoSuchFile:
            raise NoReleaserConfig()
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
                last_version_tag_name = cfg.tag_name.replace("$VERSION", last_version)
                if ws.local_tree.branch.tags.has_tag(last_version_tag_name):
                    new_version = increase_version(last_version)
                else:
                    new_version = last_version
            logging.info('Using new version: %s', new_version)

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
            update_version_in_file(ws.local_tree, update, new_version)
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
        subprocess.check_call(
            ["git", "tag", "-as", tag_name, "-m", "Release %s" % new_version],
            cwd=ws.local_tree.abspath("."),
        )
        # At this point, it's official - so let's push.
        ws.push(tags=[tag_name])
        if ws.local_tree.has_filename("setup.py"):
            subprocess.check_call(
                ["./setup.py", "sdist"], cwd=ws.local_tree.abspath(".")
            )
            from distutils.core import run_setup

            result = run_setup(ws.local_tree.abspath("setup.py"), stop_after="init")
            pypi_path = os.path.join(
                "dist", "%s-%s.tar.gz" % (result.get_name(), new_version)  # type: ignore
            )
            subprocess.check_call(
                ["twine", "upload", "--sign", pypi_path], cwd=ws.local_tree.abspath(".")
            )
        if ws.local_tree.has_filename("Cargo.toml"):
            subprocess.check_call(
                ["cargo", "upload"], cwd=ws.local_tree.abspath("."))
        for loc in cfg.tarball_location:
            subprocess.check_call(["scp", ws.local_tree.abspath(pypi_path), loc])
        if urlparse(repo_url).hostname == 'github.com':
            create_github_release(
                repo_url, tag_name, new_version, release_changes)
        # TODO(jelmer): Mark any news bugs in NEWS as fixed [later]
        # * Commit:
        #  * Update NEWS and version strings for next version
        new_pending_version = increase_version(new_version, -1)
        logging.info('Using new version %s', new_pending_version)
        if news_file:
            news_file.add_pending(new_pending_version)
            ws.local_tree.commit('Start on %s' % new_pending_version)
            ws.push()


def create_github_release(repo_url, tag_name, version, description):
    parsed_url = urlparse(repo_url)
    fullname = '/'.join(parsed_url.path.strip('/').split('/')[:2])
    token = retrieve_github_token(parsed_url.scheme, parsed_url.hostname)
    gh = Github(token)
    repo = gh.get_repo(fullname)
    logging.info('Creating release on GitHub')
    repo.create_git_release(
        tag=tag_name, name=version, draft=False, prerelease=False,
        message=description or ('Release %s.' % version))


def pypi_discover_urls():
    import xmlrpc.client
    from configparser import RawConfigParser
    client = xmlrpc.client.ServerProxy('https://pypi.org/pypi')
    cp = RawConfigParser()
    cp.read(os.path.expanduser('~/.pypirc'))
    username = cp.get('pypi', 'username')
    ret = []
    for relation, package in client.user_packages(username):
        with urlopen('https://pypi.org/pypi/%s/json' % package) as f:
            data = json.load(f)
        for key, url in data['info']['project_urls'].items():
            if key == 'Repository':
                ret.append(url)
                break
            parsed_url = urlparse(url)
            if (parsed_url.hostname == 'github.com' and
                    parsed_url.path.strip('/').count('/') == 1):
                ret.append(url)
                break
    return ret


def main(argv=None):
    import argparse

    parser = argparse.ArgumentParser("releaser")
    parser.add_argument("url", nargs="?", type=str)
    parser.add_argument(
        "--new-version", type=str, help='New version to release.')
    parser.add_argument(
        "--discover", action='store_true',
        help='Discover relevant projects to release')
    parser.add_argument(
        "--force", action="store_true",
        help='Force a new release, even if timeout is not reached.')
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO)

    if not args.discover:
        urls = [args.url or "."]
    else:
        if args.new_version:
            parser.print_usage()
            return 1
        urls = pypi_discover_urls()

    ret = 0
    for url in urls:
        if url != ".":
            logging.info('Processing %s', url)
        try:
            release_project(url, force=args.force, new_version=args.new_version)
        except RecentCommits as e:
            logging.error("Recent commits exist (%d < %d)", e.min_commit_age, e.commit_age)
            ret = 1
        except VerifyCommandFailed as e:
            logging.error('Verify command (%s) failed to run.', e.command)
            ret = 1
        except NoUnreleasedChanges:
            logging.error('No unreleased changes')
            ret = 1
        except NoReleaserConfig:
            logging.error('No configuration for releaser')
            ret = 1

    return ret


if __name__ == "__main__":
    sys.exit(main())
