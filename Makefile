PYTHON = python3

all: proto

proto: disperse/config_pb2.py

build::
	$(PYTHON) setup.py build

build-inplace:
	$(PYTHON) setup.py build_protobuf

clean:
	rm disperse/*_pb2.py

check:: flake8

check:: test

test: build
	PYTHONPATH=. pytest tests

flake8: build
	$(PYTHON) -m flake8

check:: typing

typing: build-inplace
	$(PYTHON) -m mypy disperse

docker: proto
	buildah build -t ghcr.io/jelmer/disperse:latest .
	buildah push ghcr.io/jelmer/disperse:latest
