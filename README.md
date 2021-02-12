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
 * upload to pypi, if the project is a Python project
 * create a git tag for the new release

In the future, I would like it to:

 * support more languages than just python
 * check that the CI passes for the main branch on e.g. GitHub
