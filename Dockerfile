FROM debian:sid-slim
RUN apt -y update && apt -y install brz --no-install-recommends python3-github silver-platter python3-protobuf gnupg python3-setuptools twine cython3 protobuf-compiler
ADD . /code
ENV PYTHONPATH=${PYTHONPATH}:/code
RUN cd /code/disperse && protoc --python_out=. config.proto
CMD python3 -m disperse discover --try
