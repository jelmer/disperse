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
import logging
import os
import subprocess

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
)


class UploadCommandFailed(Exception):

    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


def upload_python_artifacts(local_tree, pypi_paths):
    command = [
        "twine", "upload", "--non-interactive"] + pypi_paths
    try:
        subprocess.check_call(command, cwd=local_tree.abspath("."))
    except subprocess.CalledProcessError as e:
        raise UploadCommandFailed(command, e.returncode)


def create_setup_py_artifacts(local_tree):
    # Import setuptools, just in case it tries to replace distutils
    from distutils.core import run_setup

    import setuptools  # noqa: F401

    orig_dir = os.getcwd()
    try:
        os.chdir(local_tree.abspath('.'))
        result = run_setup(
            local_tree.abspath("setup.py"), stop_after="config")
    finally:
        os.chdir(orig_dir)
    pypi_paths = []
    is_pure = (
        not result.has_c_libraries()  # type: ignore
        and not result.has_ext_modules())  # type: ignore
    builder = ProjectBuilder(local_tree.abspath('.'))
    if is_pure:
        try:
            wheels = builder.build("wheel", output_directory=local_tree.abspath("dist"))
        except BuildBackendException as e:
            raise DistCreationFailed(e)
        pypi_paths.append(wheels)
    else:
        logging.warning(
            'python module is not pure; not uploading binary wheels')
    try:
        sdist_path = builder.build(
            "sdist", output_directory=local_tree.abspath("dist"))
    except BuildBackendException as e:
        raise DistCreationFailed(e)
    pypi_paths.append(sdist_path)
    return pypi_paths


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
