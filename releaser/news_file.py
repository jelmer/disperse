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


from . import NoUnreleasedChanges


def news_mark_released(tree, path, expected_version, release_date):
    with tree.get_file(path) as f:
        lines = list(f.readlines())
    if b'\t' in lines[0].strip():
        (version, date) = lines[0].strip().split(None, 1)
        if date != b"UNRELEASED":
            raise NoUnreleasedChanges()
    else:
        version = lines[0].strip()
    if expected_version != version.decode():
        raise AssertionError(
            "unexpected version: %s != %s" % (version, expected_version)
        )
    change_lines = []
    for line in lines[1:]:
        if (not line.strip() or line.startswith(b' ') or
                line.startswith(b'\t')):
            change_lines.append(line.decode())
        else:
            break
    lines[0] = b"%s\t%s\n" % (
        version, release_date.strftime("%Y-%m-%d").encode())
    tree.put_file_bytes_non_atomic(path, b"".join(lines))
    return ''.join(change_lines)


def news_add_pending(tree, path, new_version):
    with tree.get_file(path) as f:
        lines = list(f.readlines())
    if b' ' in lines[0] or b'\t' in lines[0]:
        lines.insert(0, b'\n')
        lines.insert(0, b"%s\t%s\n" % (new_version, b'UNRELEASED'))
    else:
        lines.insert(0, b'\n')
        lines.insert(0, b"%s\n" % (new_version, ))
    tree.put_file_bytes_non_atomic(path, b"".join(lines))


def news_find_pending(tree, path):
    with tree.get_file(path) as f:
        line = f.readline()
        if b'\t' in line.strip():
            (version, date) = line.strip().split(None, 1)
            if date != b"UNRELEASED":
                raise NoUnreleasedChanges()
        else:
            version = line.strip()
        return version.decode()
