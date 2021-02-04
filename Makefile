all: releaser/config_pb2.py

%_pb2.py: %.proto
	protoc --python_out=. $<

check:
	flake8
	PYTHONPATH=. python3 -m unittest releaser.tests.test_suite
