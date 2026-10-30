# Build from the workspace root: docker build -f crates/station/Dockerfile -t cahoots-station .
# The toolchain matches rust-toolchain.toml.
FROM rust:1.94-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p cahoots-station --locked --features objects,channels,gate

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/cahoots-station /cahoots-station
VOLUME ["/data"]
EXPOSE 8080
# Rate limits and the size cap have defaults; pass flags after the image name to change them.
ENTRYPOINT ["/cahoots-station", "--listen", "0.0.0.0:8080", "--db", "/data/station.db"]
