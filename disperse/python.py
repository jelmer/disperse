#!/usr/bin/python3
# Copyright (C) 2022 Jelmer Vernooij <jelmer@jelmer.uk>
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

__all__ = [
    'pypi_discover_urls',
    'UploadCommandFailed',
    'upload_python_artifacts',
    'create_setup_py_artifacts',
    'create_python_artifacts',
    'read_project_urls_from_pyproject_toml',
    'read_project_urls_from_setup_cfg',
    'update_version_in_pyproject_toml',
    'find_name_in_pyproject_toml',
    'find_version_in_pyproject_toml',
    'pyproject_uses_hatch_vcs',
    'find_hatch_vcs_version',
]

from build import ProjectBuilder, BuildBackendException

from . import DistCreationFailed
from ._disperse_rs import (
    update_version_in_pyproject_toml,
    pypi_discover_urls,
    find_version_in_pyproject_toml,
    find_name_in_pyproject_toml,
    pyproject_uses_hatch_vcs,
    find_hatch_vcs_version,
    read_project_urls_from_setup_cfg,
    read_project_urls_from_pyproject_toml,
    upload_python_artifacts,
    UploadCommandFailed,
    create_setup_py_artifacts,
)


def create_python_artifacts(local_tree) -> list[str]:
    pypi_paths = []
    builder = ProjectBuilder(local_tree.abspath('.'))
    try:
        wheels = builder.build("wheel", output_directory=local_tree.abspath("dist"))
    except BuildBackendException as e:
        raise DistCreationFailed(e)
    pypi_paths.append(wheels)
    try:
        sdist_path = builder.build(
            "source", output_directory=local_tree.abspath("dist"))
    except BuildBackendException as e:
        raise DistCreationFailed(e)
    pypi_paths.append(sdist_path)
    return pypi_paths
