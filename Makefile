PYTHON = python3

all: build-inplace

proto: disperse/config_pb2.py

build::
	$(PYTHON) setup.py build

build-inplace:
	$(PYTHON) setup.py build_protobuf build_rust -i

clean:
	rm disperse/*_pb2.py

check:: ruff

check:: test

test: build-inplace
	PYTHONPATH=. pytest tests

ruff:
	ruff check .

fix::
	ruff check --fix .

format::
	ruff format .

check:: typing

typing: build-inplace
	$(PYTHON) -m mypy disperse

docker: proto
	buildah build -t ghcr.io/jelmer/disperse:latest .
	buildah push ghcr.io/jelmer/disperse:latest
