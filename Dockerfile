FROM debian:sid
RUN apt -y update && apt -y install brz python3-github silver-platter python3-protobuf gnupg python3-setuptools twine cython3 protobuf-compiler mypy-protobuf
ADD . /code
ENV PYTHONPATH=${PYTHONPATH}:/code
RUN cd /code/releaser && protoc --python_out=. --mypy_out=. config.proto
CMD python3 -m releaser --discover
