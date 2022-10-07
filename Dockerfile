FROM debian:sid-slim
RUN apt -y update && apt -y install brz python3-github silver-platter python3-protobuf gnupg python3-setuptools twine cython3 protobuf-compiler mypy-protobuf
ADD . /code
RUN make -C /code proto
ENV PYTHONPATH=${PYTHONPATH}:/code
RUN cd /code/disperse && protoc --python_out=. --mypy_out=. config.proto
CMD python3 -m disperse discover --try
