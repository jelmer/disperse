FROM debian:sid
RUN apt -y update && apt -y install brz python3-github silver-platter python3-protobuf
ADD . /code
ENV PYTHONPATH=${PYTHONPATH}:/code
CMD python3 -m releaser --discover
