FROM rust:bookworm AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN cargo build --release -p mcp-web-search-server

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends chromium ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ENV CHROME_PATH=/usr/bin/chromium
COPY --from=builder /app/target/release/mcp-web-search-server /usr/local/bin/
EXPOSE 3000
ENTRYPOINT ["mcp-web-search-server"]
CMD ["--bind", "0.0.0.0:3000"]
