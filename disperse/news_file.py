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

__all__ = [
    'NewsFile',
    'check_date',
    'check_version',
    'news_find_pending',
    'OddVersion',
]

from . import NoUnreleasedChanges

from ._disperse_rs import check_date, check_version, news_find_pending, news_add_pending, OddVersion, news_mark_released


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
