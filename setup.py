#!/usr/bin/python3

from setuptools import setup

setup(
    name="releaser",
    packages=[
        "releaser",
        "releaser.tests",
    ],
    version="0.0.1",
    author="Jelmer Vernooij",
    author_email="jelmer@jelmer.uk",
    url="https://github.com/jelmer/releaser",
    description="automation for creation of releases",
    project_urls={
        "Repository": "https://github.com/jelmer/releaser.git",
    },
    entry_points={
        'console_scripts': [
            ('releaser=releaser.__main__:main'),
        ],
    },
    install_requires=['breezy', 'pygithub', 'silver_platter'],
)
