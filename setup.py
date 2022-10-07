#!/usr/bin/python3
from setuptools import setup
import setuptools.command.build
setuptools.command.build.build.sub_commands.append(
    ('build_proto', lambda x: True))
setup()
