FROM debian:sid
RUN apt -y update && apt -y install brz python3-github silver-platter
ADD . /code
ENV PYTHONPATH=${PYTHONPATH}:/code
CMD python3 -m releaser --discover
