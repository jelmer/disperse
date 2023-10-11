#!/usr/bin/python3
from setuptools import setup
from setuptools_protobuf import Protobuf
from setuptools_rust import Binding, RustExtension

setup(
    protobufs=[Protobuf('disperse/config.proto', mypy=True)],
    rust_extensions=[RustExtension("disperse._disperse_rs",
                                   "disperse-py/Cargo.toml",
                                   binding=Binding.PyO3)])
