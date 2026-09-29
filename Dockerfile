FROM rust:1.94.0-slim-bookworm AS build
WORKDIR /build
COPY Cargo.toml Cargo.lock README.md LICENSE ./
COPY src ./src
RUN cargo build --release --locked --bin sleepapi

FROM debian:bookworm-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/givtaj/sleepapi"
LABEL org.opencontainers.image.description="Configurable HTTP latency and failure simulation API"
LABEL org.opencontainers.image.licenses="MIT"
COPY --from=build /build/target/release/sleepapi /usr/local/bin/sleepapi
USER 65532:65532
ENV SLEEPAPI_ADDR=0.0.0.0:3000
EXPOSE 3000
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/sleepapi"]
