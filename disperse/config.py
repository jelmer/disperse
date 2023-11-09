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

import os
import toml


CONFIG_FILE_NAME = "disperse.toml"


def load_config():
    # Check if XDG_CONFIG_HOME is set, otherwise use the default path
    xdg_config_home = os.environ.get(
        "XDG_CONFIG_HOME", os.path.join(os.path.expanduser("~"), ".config")
    )

    config_file_path = os.path.join(xdg_config_home, "disperse", CONFIG_FILE_NAME)

    if not os.path.exists(config_file_path):
        return {}

    with open(config_file_path) as f:
        return toml.load(f)
