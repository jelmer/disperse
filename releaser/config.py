from google.protobuf import text_format

from . import config_pb2
Project = config_pb2.Project


def read_config(f):
    return text_format.Parse(f.read(), config_pb2.Config())


def read_project(f):
    return text_format.Parse(f.read(), config_pb2.Project())
