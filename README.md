Releaser
========

Releaser is a simple script that I use to create releases for some of the free
software packages I maintain. It's meant to streamline the releasing process,
reducing the human effort involved in creating a release as well as the
likelihood of a brown-bag release.

It can do one or more of the following:

 * derive the intended new version by checking existing releases and
   version strings specified in project files
 * update NEWS files with the release date
 * make sure various files contain the correct version string
 * verify that the testsuite runs successfully
 * optionally only create a release if there were no changes to the repository
   in the last X days (useful for running in a cronjob)
 * upload to a repository site:
    * pypi, if the project is a Python project
    * crates.io, if the project is a rust project
 * create a git tag for the new release
 * create "release" entries on GitHub

Configuration
-------------

To a large extent, releaser will automatically figure out what needs to happen.
It can discover the projects you maintain on pypi by reading ~/.pypirc for your
username and enumerating them.

It can parse and modify setup.py and Cargo.toml files.

It uses a configuration file (releaser.conf) for anything that can not be
autodetected, and which lives in the repository root.

For example:

```

   tag_format: "dulwich-%(release)s"
   news_path: "NEWS"

```

Running from docker
-------------------

The easiest way to run releaser is to use the docker image at
``ghcr.io/jelmer/releaser``. You'll need to make sure that appropriate SSH
and PGP keys are available.

The author regularly runs releaser inside of a Kubernetes cronjob.

Future
------

In the future, I would like it to:

 * support more languages than just python and rust
 * check that the CI passes for the main branch on e.g. GitHub
