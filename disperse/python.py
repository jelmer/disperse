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

from breezy.tree import Tree
from breezy.workingtree import WorkingTree
from build import ProjectBuilder, BuildBackendException
import json
import logging
import os
import subprocess
from typing import Optional
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from . import version_string, DistCreationFailed
from ._disperse_rs import (
    update_version_in_pyproject_toml,
    pypi_discover_urls,
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


def read_project_urls_from_pyproject_toml(path):
    from toml.decoder import load
    with open(path) as f:
        d = load(f)
    project_urls = d.get('project', {}).get('urls', {})
    for key in ['GitHub', 'Source Code', 'Repository']:
        try:
            yield (project_urls[key], 'HEAD')
        except KeyError:
            pass


def read_project_urls_from_setup_cfg(path):
    import setuptools.config.setupcfg
    config = setuptools.config.setupcfg.read_configuration(path)
    metadata = config.get('metadata', {})
    project_urls = metadata.get('project_urls', {})
    for key in ['GitHub', 'Source Code', 'Repository']:
        try:
            yield (project_urls[key], 'HEAD')
        except KeyError:
            pass


def find_name_in_pyproject_toml(tree: Tree) -> Optional[str]:
    from toml.decoder import loads
    d = loads(tree.get_file_text('pyproject.toml').decode('utf-8'))
    return d.get('project', {}).get('name')


def find_version_in_pyproject_toml(tree: Tree) -> Optional[str]:
    from toml.decoder import loads
    d = loads(tree.get_file_text('pyproject.toml').decode('utf-8'))
    return d.get('project', {}).get('version')


def pyproject_uses_hatch_vcs(tree: Tree) -> bool:
    from toml.decoder import loads
    d = loads(tree.get_file_text('pyproject.toml').decode('utf-8'))
    source = d.get('tool', {}).get('hatch', {}).get('version', {}).get("source")
    return source == "vcs"


def find_hatch_vcs_version(tree: WorkingTree) -> Optional[str]:
    cwd = tree.abspath(".")
    output = subprocess.check_output(["hatchling", "version"], cwd=cwd)
    version = output.strip().decode()
    tupled_version = tuple(int(x) for x in version.split(".")[:3])
    return "%d.%d.%d" % tupled_version
