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
from github import Github  # type: ignore
import logging
import os
import re
import subprocess
import sys
from urllib.parse import urlparse

import breezy.git
import breezy.bzr  # noqa: F401
from breezy.branch import Branch
from silver_platter.workspace import Workspace


class NoUnreleasedChanges(Exception):
    """No unreleased changes."""


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


def increase_version(version, idx=-1):
    parts = [int(x) for x in version.split('.')]
    parts[idx] += 1
    return '.'.join(map(str, parts))



def find_pending_version(tree, cfg):
    if cfg.news_file:
        with tree.get_file(cfg.news_file) as f:
            line = f.readline()
            (version, date) = line.strip().split(None, 1)
            if date != b"UNRELEASED":
                raise NoUnreleasedChanges()
            return version.decode()
    else:
        raise NotImplementedError


def news_mark_released(tree, path, expected_version):
    with tree.get_file(path) as f:
        lines = list(f.readlines())
    (version, date) = lines[0].strip().split(None, 1)
    if date != b"UNRELEASED":
        raise NoUnreleasedChanges()
    if expected_version != version.decode():
        raise AssertionError(
            "unexpected version: %s != %s" % (version, expected_version)
        )
    change_lines = []
    for line in lines[1:]:
        if line.startswith(b' ') or line.startswith(b'\t'):
            change_lines.append(line.decode())
        else:
            break
    lines[0] = b"%s\t%s\n" % (version, datetime.now().strftime("%Y-%m-%d").encode())
    tree.put_file_bytes_non_atomic(path, b"".join(lines))
    return ''.join(change_lines)


def news_add_pending(tree, path, new_version):
    with tree.get_file(path) as f:
        lines = list(f.readlines())
    lines.insert(0, b'\n')
    lines.insert(0, b"%s\t%s\n" % (new_version, 'UNRELEASED'))
    tree.put_file_bytes_non_atomic(path, b"".join(lines))


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


def check_release_age(branch, cfg):
    rev = branch.repository.get_revision(branch.last_revision())
    if cfg.timeout_days is not None:
        commit_time = datetime.fromtimestamp(rev.timestamp)
        time_delta = datetime.now() - commit_time
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
                    return m.group(1).decode()
        raise KeyError
    else:
        raise NotImplementedError


def release_project(repo_url, force=False, new_version=None):
    from .config import read_project

    branch = Branch.open(repo_url)
    with Workspace(branch) as ws:
        cfg = read_project(ws.local_tree.get_file("releaser.conf"))
        try:
            check_release_age(ws.local_tree.branch, cfg)
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

        if cfg.verify_command:
            try:
                subprocess.check_call(
                    cfg.verify_command, cwd=ws.local_tree.abspath("."), shell=True
                )
            except subprocess.CalledProcessError as e:
                raise VerifyCommandFailed(cfg.verify_command, e.returncode)

        logging.info("releasing %s", new_version)
        if cfg.news_file:
            release_changes = news_mark_released(ws.local_tree, cfg.news_file, new_version)
        else:
            release_changes = None
        for update in cfg.update_version:
            update_version_in_file(ws.local_tree, update, new_version)
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
                "dist", "%s-%s.tar.gz" % (result.get_name(), new_version)
            )
            subprocess.check_call(
                ["twine", "upload", "--sign", pypi_path], cwd=ws.local_tree.abspath(".")
            )
        for loc in cfg.tarball_location:
            subprocess.check_call(["scp", ws.local_tree.abspath(pypi_path), loc])
        if urlparse(repo_url).hostname == 'github.com':
            create_github_release(repo_url, tag_name, new_version, release_changes)
        # TODO(jelmer): Mark any news bugs in NEWS as fixed [later]
        # * Commit:
        #  * Update NEWS and version strings for next version
        new_pending_version = increase_version(new_version, -1)
        logging.info('Using new version %s', new_pending_version)
        if cfg.news_file:
            news_add_pending(ws.local_tree, cfg.news_file, new_pending_version)


def create_github_release(repo_url, tag_name, version, description):
    parsed_url = urlparse(repo_url)
    fullname = '/'.join(parsed_url.path.strip('/').split('/')[:2])
    gh = Github()
    repo = gh.get_repo(fullname)
    repo.create_git_release(
        tag=tag_name, name=version, draft=False, prerelease=False,
        message=description or ('Release %s.' % version))


def main(argv=None):
    import argparse

    parser = argparse.ArgumentParser("releaser")
    parser.add_argument("url", nargs="?", type=str)
    parser.add_argument("--new-version", type=str)
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    logging.basicConfig(level=logging.INFO)

    try:
        release_project(args.url or ".", force=args.force, new_version=args.new_version)
    except RecentCommits as e:
        logging.error("Recent commits exist (%d < %d)", e.min_commit_age, e.commit_age)
        return 1
    except VerifyCommandFailed as e:
        logging.error('Verify command (%s) failed to run.', e.command)
        return 1
    except NoUnreleasedChanges:
        logging.error('No unreleased changes')
        return 1

    return 0


if __name__ == "__main__":
    sys.exit(main())
