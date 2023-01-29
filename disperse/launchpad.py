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


import datetime
import logging
import os

from launchpadlib.launchpad import Launchpad


def find_project_series(project, series_name=None, target_version=None):
    if series_name is not None:
        try:
            [series] = [s for s in project.series if s.name == series_name]
        except IndexError:
            raise KeyError(series_name)

    if len(project.series) == 1:
        return project.series[0]
    elif len(project.series) > 1:
        possible_series = [
            s for s in project.series
            if target_version and target_version.startswith(s.name)]
        if len(possible_series) == 1:
            return possible_series[0]
        else:
            logging.warning(
                'Multiple release series exist, but none specified. '
                'Assuming development focus')
            return project.development_focus
    else:
        raise AssertionError("no release series for %r" % project)


def create_milestone(project, version, series_name=None):
    series = find_project_series(project, series_name)
    release_date = datetime.date.today().strftime('%Y-%m-%d')
    return series.newMilestone(
        name=version, date_targeted=release_date)


def get_project(project):
    launchpad = Launchpad.login_with("disperse", "production", version="devel")

    # Look up the project using the Launchpad instance.
    return launchpad.projects[project]


def find_release(project, release):
    for rel in project.releases:
        if rel.version == release:
            return rel
    return None


def create_release_from_milestone(project, version):
    for milestone in project.all_milestones:
        if milestone.name == version:
            today = datetime.date.today().strftime('%Y-%m-%d')
            return milestone.createProductRelease(date_released=today)
    return None


def ensure_release(proj, version, series_name=None, release_notes=None):
    release = proj.find_release(proj, version)
    if not release:
        release = create_release_from_milestone(proj, version)
    if not release:
        milestone = create_milestone(proj, version, series_name=series_name)
        today = datetime.date.today().strftime('%Y-%m-%d')
        return milestone.createProductRelease(date_released=today)

    if release_notes:
        release.release_notes = release_notes
    return release


def add_release_files(release, artifacts):
    for artifact in artifacts:
        if artifact.endswith('.tar.gz'):
            with open(artifact, 'rb') as f:
                release.add_file(
                    filename=os.path.basename(artifact),
                    description='release tarball',
                    content_type='application/x-gzip',
                    file_type='Code Release Tarball',
                    file_content=f.read())
