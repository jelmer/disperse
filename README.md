disperse
========

disperse is a simple script that I use to create releases for some of the free
software packages I maintain. It's meant to streamline the releasing process,
reducing the human effort involved in creating a release as well as the
likelihood of a brown-bag release.

It can do one or more of the following:

 * check if CI is currently passing (for supported platforms, like GitHub)
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
 * create "release" entries (on supported platforms, like GitHub)

Disperse was previously known as "releaser".

Configuration
-------------

To a large extent, disperse will automatically figure out what needs to happen.
It can discover the projects you maintain on pypi by reading ~/.pypirc for your
username and enumerating them.

It can parse and modify setup.py and Cargo.toml files.

It uses a configuration file (disperse.conf) for anything that can not be
autodetected, and which lives in the repository root.

For example:

```

   tag_format: "dulwich-%(release)s"
   news_path: "NEWS"

```

Basic usage
-----------

disperse has various subcommands. The core ones are:

 * release - create a new release for project in $CWD or at a specific URL
 * discover - find projects that the current user owns (e.g. on pypi) and
      release them if they have unreleased changes and are significant enough
 * validate - validate the disperse configuration

Running from docker
-------------------

The easiest way to run disperse is to use the docker image at
``ghcr.io/jelmer/disperse``. You'll need to make sure that appropriate SSH
and PGP keys are available.

The author regularly runs disperse inside of a Kubernetes cronjob.

Future
------

In the future, I would like it to:

 * support more languages than just python and rust
