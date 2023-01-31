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

import re
from datetime import datetime
from typing import Optional, Tuple

from breezy.mutabletree import MutableTree
from breezy.tree import Tree

from . import NoUnreleasedChanges


class NewsFile:

    def __init__(self, tree, path):
        self.tree = tree
        self.path = path

    def mark_released(self, expected_version, release_date):
        return news_mark_released(
            self.tree, self.path, expected_version, release_date)

    def add_pending(self, new_version):
        return news_add_pending(self.tree, self.path, new_version)

    def find_pending(self):
        return news_find_pending(self.tree, self.path)

    def validate(self):
        try:
            self.find_pending()
        except NoUnreleasedChanges:
            pass


def news_mark_released(
        tree, path: str, expected_version: str, release_date: datetime):
    lines = tree.get_file_lines(path)
    i = skip_header(lines)
    version, date, line_format, pending = parse_version_line(lines[i])
    if not pending:
        raise NoUnreleasedChanges()
    if version is not None and expected_version != version:
        raise AssertionError(
            "unexpected version: {} != {}".format(version, expected_version)
        )
    change_lines = []
    for line in lines[i+1:]:
        if (not line.strip() or line.startswith(b' ') or
                line.startswith(b'\t')):
            change_lines.append(line.decode())
        else:
            break
    lines[i] = (line_format % {
        'version': expected_version,
        'date': release_date.strftime("%Y-%m-%d")}).encode() + b'\n'
    tree.put_file_bytes_non_atomic(path, b"".join(lines))
    return ''.join(change_lines)


class PendingExists(Exception):
    """Last item is already pending."""


def news_add_pending(tree: MutableTree, path: str, new_version: str):
    assert new_version
    lines = tree.get_file_lines(path)
    i = skip_header(lines)
    unused_version, unused_date, line_format, pending = (
        parse_version_line(lines[i]))
    if pending:
        raise PendingExists(unused_version, unused_date)
    lines.insert(i, b'\n')
    lines.insert(i, (line_format % {
        'version': new_version,
        'date': 'UNRELEASED'}).encode() + b"\n")
    tree.put_file_bytes_non_atomic(path, b"".join(lines))


def skip_header(lines):
    i = 0
    for i, line in enumerate(lines):
        if line.startswith(b'Changelog for '):
            continue
        if re.fullmatch(b'.* release notes', line.strip()):
            continue
        if all([x in ('=', '-') for x in line.strip().decode()]):
            continue
        if not line.strip():
            continue
        return i
    raise ValueError('no contents in news file?')


class OddVersion(Exception):
    """Version string found was odd."""

    def __init__(self, version):
        self.version = version


def check_version(v: str) -> bool:
    import re
    if v == "UNRELEASED" or v == "%(version)s" or v == 'NEXT':
        return True
    if not re.fullmatch(r'[0-9\.]+', v):
        raise OddVersion(v)
    return False


def check_date(d):
    if d == "UNRELEASED" or d.startswith('NEXT '):
        return True
    return False


def parse_version_line(line: bytes) -> Tuple[
        Optional[str], Optional[str], str, bool]:
    """Extract version info from news line.

    Args:
      line: Line to parse
    Returns:
      tuple with version, date released, line template, is_pending
    """
    if b'\t' in line.strip():
        (version, date) = line.strip().split(b'\t', 1)
        version_is_placeholder = check_version(version.decode())
        date_is_placeholder = check_date(date.decode())
        pending = version_is_placeholder or date_is_placeholder
        return (
            version.decode() if not version_is_placeholder else None,
            date.decode() if not date_is_placeholder else None,
            '%(version)s\t%(date)s', pending)
    if b' ' in line.strip():
        (version, date) = line.strip().split(b' ', 1)
        if date.startswith(b'(') and date.endswith(b')'):
            date = date[1:-1]
            template = '%(version)s (%(date)s)'
        else:
            template = '%(version)s %(date)s'

        assert version
        version_is_placeholder = check_version(version.decode())
        date_is_placeholder = check_date(date.decode())
        pending = version_is_placeholder or date_is_placeholder
        return (
            version.decode() if not version_is_placeholder else None,
            date.decode() if not date_is_placeholder else None, template,
            pending)
    else:
        version = line.strip()
        pending = date_is_placeholder = check_version(version.decode())
        return (
            version.decode() if not date_is_placeholder else None, None,
            '%(version)s', pending)


def news_find_pending(tree: Tree, path: str) -> Optional[str]:
    """Find pending version in news file.

    Args:
      tree: Tree object
      path: Path to news file in tree
    Returns:
      version string
    """
    lines = tree.get_file_lines(path)
    i = skip_header(lines)
    (version, date, line_format, pending) = parse_version_line(lines[i])
    if not pending:
        raise NoUnreleasedChanges()
    return version
