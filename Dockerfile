FROM debian:sid-slim AS build
# Add the experimental repository to apt sources
RUN echo "deb http://deb.debian.org/debian experimental main" > /etc/apt/sources.list.d/experimental.list
RUN apt -y update && apt -y install --no-install-recommends cargo ca-certificates python3-all-dev libssl-dev pkg-config protobuf-compiler
# We need rustc 1.86, which is in the experimental repository
RUN apt -y install -t experimental rustc
ADD . /code
RUN cd /code && cargo build --release
FROM debian:sid-slim
COPY --from=build /code/target/release/disperse /code/bin/disperse
ENTRYPOINT ["/code/bin/disperse"]
CMD ["discover", "--try"]
