FROM debian:sid-slim
RUN apt -y update && apt -y install brz --no-install-recommends gnupg python3-setuptools python3-pip twine protobuf-compiler git openssh-client tox npm
ADD . /code
RUN pip3 install /code
ENV PROTOCOL_BUFFERS_PYTHON_IMPLEMENTATION=python
ENTRYPOINT ["python3", "-m", "disperse"]
CMD ["discover", "--try"]
