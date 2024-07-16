FROM debian:sid-slim AS build
RUN apt -y update && apt -y install --no-install-recommends cargo ca-certificates python3-all-dev libssl-dev pkg-config protobuf-compiler
ADD . /code
RUN cd /code && cargo build --release
FROM debian:sid-slim
COPY --from=build /code/target/release/disperse /code/bin/disperse
ENTRYPOINT ["/code/bin/disperse"]
CMD ["discover", "--try"]
