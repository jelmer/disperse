from datetime import datetime
import logging
import os
import re
import subprocess
import sys

import breezy.git
import breezy.bzr
from breezy.branch import Branch
from silver_platter.workspace import Workspace


class NoUnreleasedChanges(Exception):
    """No unreleased changes."""


def check_ready(project):
    pass
    # TODO(jelmer): Check if CI state is green
    # TODO(jelmer): Check timeout


def find_pending_version(tree, cfg):
    if cfg.news_file:
        with tree.get_file(cfg.news_file) as f:
            l = f.readline()
            (version, date) = l.strip().split(None, 1)
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
            'unexpected version: %s != %s' % (version, expected_version))
    lines[0] = b'%s\t%s\n' % (
        version, datetime.now().strftime('%Y-%m-%d').encode())
    tree.put_file_bytes_non_atomic(path, b''.join(lines))


def update_version_in_file(tree, update_cfg, new_version):
    with tree.get_file(update_cfg.path) as f:
        lines = list(f.readlines())
    r = re.compile(update_cfg.match.encode())
    for i, line in enumerate(lines):
        if not r.match(line):
            continue
        tupled_version = (
            '(%s)' % ', '.join(new_version.split('.')))
        lines[i] = update_cfg.new_line.encode().replace(
            b'$VERSION', new_version.encode()).replace(
            b'$TUPLED_VERSION', tupled_version.encode())


def release_project(repo_url):
    from .config import read_project
    branch = Branch.open(repo_url)
    with Workspace(branch) as ws:
        cfg = read_project(ws.local_tree.get_file('releaser.conf'))
        new_version = find_pending_version(ws.local_tree, cfg)
        logging.info('%s: releasing %s', cfg.name, new_version)
        if cfg.news_file:
            news_mark_released(
                ws.local_tree, cfg.news_file, new_version)
        for update in cfg.update_version:
            update_version_in_file(ws.local_tree, update, new_version)
        ws.local_tree.commit('Release %s.' % new_version)
        tag_name = cfg.tag_format % new_version
        subprocess.check_call(
            ['git', 'tag', '-as', tag_name, '-m', 'Release %s' % new_version],
            cwd=ws.local_tree.abspath('.'))
        # * Tag && sign tag
        if ws.local_tree.has_filename('setup.py'):
            subprocess.check_call(
                ['./setup.py', 'dist'], cwd=ws.local_tree.abspath('.'))
            pypi_path = os.path.join(
                'dist', '%s-%s.tar.gz' % (cfg.pypi_name, new_version))
        if cfg.pypi_name:
            subprocess.check_call(
                ['twine', 'upload', '--sign', pypi_path],
                cwd=ws.local_tree.abspath('.'))
        for loc in cfg.tarball_location:
            subprocess.check_call(
                ['scp', ws.local_tree.abspath(pypi_path), loc])
        # TODO(jelmer): Mark any news bugs in NEWS as fixed [later]
        # * Commit:
        #  * Update NEWS and version strings for next version
        ws.push()



def main(argv=None):
    import argparse
    parser = argparse.ArgumentParser('releaser')
    parser.add_argument('url', nargs='?', type=str)
    args = parser.parse_args()

    if args.url:
        release_project(args.url)
    else:
        release_project('.')


if __name__ == '__main__':
    sys.exit(main())
