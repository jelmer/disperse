FROM debian:sid-slim
RUN apt -y update && apt -y install python3-venv brz --no-install-recommends gnupg python3-setuptools python3-pip twine protobuf-compiler git openssh-client tox npm mypy-protobuf build-essential make flake8 mypy python3-all-dev sassc python3-yaml python3-sphinx libpcre3-dev python3-pytest
ADD . /code
RUN python3 -m venv /code && /code/bin/pip3 install /code
ENV PROTOCOL_BUFFERS_PYTHON_IMPLEMENTATION=python
ENTRYPOINT ["/code/bin/python3", "-m", "disperse"]
CMD ["discover", "--try"]
