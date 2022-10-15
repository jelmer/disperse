FROM debian:sid-slim
RUN apt -y update && apt -y install brz --no-install-recommends python3-github silver-platter python3-protobuf gnupg python3-setuptools python3-pip twine protobuf-compiler git
ADD . /code
RUN pip3 install /code
CMD python3 -m disperse discover --try
