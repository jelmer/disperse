FROM debian:sid-slim
RUN apt -y update && apt -y install brz --no-install-recommends python3-github silver-platter python3-protobuf gnupg python3-setuptools python3-pip twine protobuf-compiler git openssh-client tox
ADD . /code
RUN pip3 install "setuptools-protobuf[mypy]" && pip3 install /code git+https://github.com/breezy-team/breezy
ENV PROTOCOL_BUFFERS_PYTHON_IMPLEMENTATION=python
CMD python3 -m disperse discover --try
