all: releaser/config_pb2.py

%_pb2.py: %.proto
	protoc --python_out=. --mypy_out=. $<

clean:
	rm releaser/*_pb2.py

check:
	flake8
	PYTHONPATH=. python3 -m unittest releaser.tests.test_suite

docker:
	docker build -t ghcr.io/jelmer/releaser .
	docker push ghcr.io/jelmer/releaser
