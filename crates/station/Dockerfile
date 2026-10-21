# Build from the workspace root: docker build -f crates/station/Dockerfile -t cahoots-station .
FROM rust:1.88-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p cahoots-station --locked

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/cahoots-station /cahoots-station
VOLUME ["/data"]
EXPOSE 8080
ENTRYPOINT ["/cahoots-station", "--listen", "0.0.0.0:8080", "--log", "/data/station.jsonl"]
