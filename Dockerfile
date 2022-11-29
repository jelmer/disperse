FROM debian:sid-slim
RUN apt -y update && apt -y install brz --no-install-recommends python3-github silver-platter python3-protobuf gnupg python3-setuptools python3-pip twine protobuf-compiler git openssh-client tox npm
ADD . /code
RUN pip3 install "setuptools-protobuf[mypy]" git+https://github.com/breezy-team/breezy && pip3 install /code
ENV PROTOCOL_BUFFERS_PYTHON_IMPLEMENTATION=python
CMD python3 -m disperse discover --try
