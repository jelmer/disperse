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

from google.protobuf import text_format  # type: ignore

from breezy.transport import NoSuchFile
from breezy.tree import Tree

from . import config_pb2

ProjectConfig = config_pb2.Project


def read_project(f) -> ProjectConfig:
    return text_format.Parse(f.read(), config_pb2.Project())


def read_project_with_fallback(tree: Tree) -> ProjectConfig:
    try:
        with tree.get_file("disperse.conf") as f:
            return read_project(f)
    except NoSuchFile as orig:
        try:
            with tree.get_file("releaser.conf") as f:
                return read_project(f)
        except NoSuchFile:
            raise orig
