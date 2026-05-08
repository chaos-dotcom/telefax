# Build stage
FROM rust:1.95-slim AS builder

WORKDIR /app

# Install dependencies needed for building (libssl-dev for reqwest, pkg-config)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifest and cache dependencies
COPY Cargo.toml Cargo.lock ./
COPY src ./src

# Build release binary
RUN cargo build --release

# Runtime stage
FROM debian:bookworm-slim

WORKDIR /app

# Install CUPS client and image libraries for runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    cups-client \
    libcups2 \
    libjpeg62-turbo \
    libpng16-16 \
    libtiff6 \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the release binary from builder
COPY --from=builder /app/target/release/telefax ./telefax

# Note: .env file is NOT copied. It should be provided at runtime via docker-compose env_file or similar.

CMD ["./telefax"]
