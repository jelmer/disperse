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

import subprocess

from breezy.workingtree import WorkingTree
from breezy.tree import Tree


def cargo_publish(tree, subpath="."):
    subprocess.check_call(["cargo", "publish"], cwd=tree.abspath(subpath))


def update_version_in_cargo(tree: WorkingTree, new_version: str) -> None:
    from toml.decoder import TomlPreserveCommentDecoder, load
    from toml.encoder import TomlPreserveCommentEncoder, dumps

    with tree.get_file('Cargo.toml') as f:
        d = load(f, dict, TomlPreserveCommentDecoder())
    d['package']['version'] = new_version
    tree.put_file_bytes_non_atomic(
        'Cargo.toml',
        dumps(d, TomlPreserveCommentEncoder()).encode())  # type: ignore
    subprocess.check_call(['cargo', 'update'], cwd=tree.abspath('.'))


def find_version_in_cargo(tree: Tree) -> str:
    from toml.decoder import TomlPreserveCommentDecoder, load

    with tree.get_file('Cargo.toml') as f:
        d = load(f, dict, TomlPreserveCommentDecoder())
    return d['package']['version']
