# ==============================================================================
# Stage 1: Rust Workspace Builder
# ==============================================================================
FROM rust:1.85-bookworm AS builder

WORKDIR /build

# Copy Cargo workspace manifests and source crates
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

# Build release binaries (LTO, stripped, single codegen-unit)
RUN cargo build --release --workspace --bins

# ==============================================================================
# Stage 2: Minimal Hardened Runtime Image
# ==============================================================================
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy compiled binaries from builder stage
COPY --from=builder /build/target/release/silas-blue /usr/local/bin/silas-blue
COPY --from=builder /build/target/release/silas-migrate /usr/local/bin/silas-migrate

# Create runtime directories with restricted permissions
RUN mkdir -p /app/data /app/backups && chmod 700 /app/data /app/backups

ENV SILAS_HEADLESS=true \
    SILAS_WEB_HOST=0.0.0.0 \
    SILAS_WEB_PORT=8080 \
    SILAS_DB_PATH=/app/data/silas.db \
    SILAS_BACKUPS_DIR=/app/backups

EXPOSE 8080

CMD ["silas-blue", "--headless", "--web-host", "0.0.0.0", "--web-port", "8080"]
