all: proto

proto: disperse/config_pb2.py

%_pb2.py: %.proto
	protoc --python_out=. --mypy_out=. $<

clean:
	rm disperse/*_pb2.py

check:
	flake8
	PYTHONPATH=. python3 -m unittest disperse.tests.test_suite

docker: proto
	docker build -t ghcr.io/jelmer/disperse .
	docker push ghcr.io/jelmer/disperse
