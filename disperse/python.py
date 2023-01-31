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


import json
import logging
import os
import subprocess
import sys
from glob import glob
from urllib.parse import urlparse
from urllib.request import Request, urlopen

from . import version_string


class DistCommandFailed(Exception):

    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


class UploadCommandFailed(Exception):

    def __init__(self, command, retcode):
        self.command = command
        self.retcode = retcode


def pypi_discover_urls(pypi_user):
    import xmlrpc.client
    client = xmlrpc.client.ServerProxy('https://pypi.org/pypi')
    ret = []
    for relation, package in client.user_packages(pypi_user):  # type: ignore
        req = Request(
            f'https://pypi.org/pypi/{package}/json',
            headers={'Content-Type': f'disperse/{version_string}'})
        with urlopen(req) as f:
            data = json.load(f)
        project_urls = data['info']['project_urls']
        if project_urls is None:
            logging.warning(f'Project {package} does not have project URLs')
            continue
        for key, url in project_urls.items():
            if key == 'Repository':
                ret.append(url)
                break
            parsed_url = urlparse(url)
            if (parsed_url.hostname == 'github.com' and
                    parsed_url.path.strip('/').count('/') == 1):
                ret.append(url)
                break
    return ret


def upload_python_artifacts(local_tree, pypi_paths):
    command = [
        "twine", "upload", "--non-interactive",
        "--sign"] + pypi_paths
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
    if is_pure:
        try:
            subprocess.check_call(
                ["./setup.py", "egg_info", "-Db", "", "bdist_wheel"],
                cwd=local_tree.abspath(".")
            )
        except subprocess.CalledProcessError as e:
            raise DistCommandFailed(
                "setup.py bdist_wheel", e.returncode)
        wheels_glob = 'dist/{}-{}-*-any.whl'.format(
            result.get_name().replace('-', '_'),  # type: ignore
            result.get_version())  # type: ignore
        wheels = glob(
            os.path.join(local_tree.abspath('.'), wheels_glob))
        if not wheels:
            raise AssertionError(
                'setup.py bdist_wheel did not produce expected files. '
                'glob: %r, files: %r' % (
                    wheels_glob,
                    os.listdir(local_tree.abspath('dist'))))
        pypi_paths.extend(wheels)
    else:
        logging.warning(
            'python module is not pure; not uploading binary wheels')
    try:
        subprocess.check_call(
            ["./setup.py", "egg_info", "-Db", "", "sdist"],
            cwd=local_tree.abspath(".")
        )
    except subprocess.CalledProcessError as e:
        raise DistCommandFailed("setup.py sdist", e.returncode)
    sdist_path = os.path.join(
        "dist", "{}-{}.tar.gz".format(
            result.get_name(), result.get_version()))  # type: ignore
    pypi_paths.append(sdist_path)
    return pypi_paths


def create_python_artifacts(local_tree):
    # Import setuptools, just in case it tries to replace distutils
    import setuptools  # noqa: F401
    from setuptools.config.setupcfg import read_configuration

    config = read_configuration(local_tree.abspath('setup.cfg'))
    pypi_paths = []
    try:
        subprocess.check_call(
            [sys.executable, "-m", "build", "-w"],
            cwd=local_tree.abspath(".")
        )
    except subprocess.CalledProcessError as e:
        raise DistCommandFailed(
            "setup.py bdist_wheel", e.returncode)
    wheels_glob = 'dist/{}-{}-*-any.whl'.format(
        config['metadata']['name'].replace('-', '_'),
        config['metadata']['version'])
    wheels = glob(
        os.path.join(local_tree.abspath('.'), wheels_glob))
    if not wheels:
        raise AssertionError(
            'setup.py bdist_wheel did not produce expected files. '
            'glob: %r, files: %r' % (
                wheels_glob,
                os.listdir(local_tree.abspath('dist'))))
    pypi_paths.extend(wheels)
    try:
        subprocess.check_call(
            [sys.executable, "-m", "build", "--sdist"],
            cwd=local_tree.abspath(".")
        )
    except subprocess.CalledProcessError as e:
        raise DistCommandFailed("setup.py sdist", e.returncode)
    sdist_path = os.path.join(
        "dist", "{}-{}.tar.gz".format(
            config['metadata']['name'],
            config['metadata']['version']))
    pypi_paths.append(sdist_path)
    return pypi_paths


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
