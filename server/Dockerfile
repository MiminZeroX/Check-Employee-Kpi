# Build context: repository root  (docker build -f server/Dockerfile .)
FROM rust:1-bookworm AS build
WORKDIR /src
COPY server/Cargo.toml server/Cargo.lock server/build.rs ./
# cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs && cargo build --release && rm -rf src
COPY server/src ./src
COPY server/migrations ./migrations
COPY server/build.rs ./build.rs
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates tzdata && rm -rf /var/lib/apt/lists/* \
    && useradd -r -u 10001 smartoffice
COPY --from=build /src/target/release/smartoffice-server /usr/local/bin/smartoffice-server
COPY web /srv/web
ENV WEB_DIR=/srv/web BIND_ADDR=0.0.0.0:8080 RUST_LOG=info
USER smartoffice
EXPOSE 8080
CMD ["smartoffice-server"]
