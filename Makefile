all: proto

proto: disperse/config_pb2.py

%_pb2.py: %.proto
	protoc --python_out=. --mypy_out=. $<

clean:
	rm disperse/*_pb2.py

check:: flake8

check:: test

test:
	PYTHONPATH=. python3 -m unittest disperse.tests.test_suite

flake8:
	flake8

check:: typing

typing:
	mypy disperse

docker: proto
	buildah build -t ghcr.io/jelmer/disperse:latest .
	buildah push ghcr.io/jelmer/disperse:latest
