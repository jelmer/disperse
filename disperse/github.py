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
from urllib.parse import urlparse
import time

from github import Github  # type: ignore
from breezy.plugins.github.forge import retrieve_github_token


DEFAULT_GITHUB_CI_TIMEOUT = 60 * 24


def get_github_repo(repo_url: str):
    if repo_url.endswith('.git'):
        repo_url = repo_url[:-4]
    parsed_url = urlparse(repo_url)
    fullname = '/'.join(parsed_url.path.strip('/').split('/')[:2])
    try:
        token = retrieve_github_token(  # type: ignore
            parsed_url.scheme, parsed_url.hostname)
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
    for check in commit.get_check_runs():
        if check.conclusion in ('success', 'skipped'):
            continue
        elif check.conclusion is None:
            raise GitHubStatusPending(check.head_sha, check.html_url)
        else:
            raise GitHubStatusFailed(check.head_sha, check.html_url)


def wait_for_gh_actions(repo, committish, *, timeout=DEFAULT_GITHUB_CI_TIMEOUT):
    logging.info('Waiting for CI for %s on %s to go green', repo, committish)
    if not committish:
        committish = 'HEAD'
    commit = repo.get_commit(committish)
    start_time = time.time()
    while time.time() - start_time < timeout:
        for check in commit.get_check_runs():
            if check.conclusion in ("success", "SKipped"):
                continue
            elif check.conclusion == "pending":
                time.sleep(30)
                break
            else:
                raise GitHubStatusFailed(check.head_sha, check.html_url)
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
