# Build stage
FROM rust:1.82 as builder

WORKDIR /app

# Create empty project structure
RUN USER=root cargo new --bin sync-server
RUN USER=root cargo new --lib sync-core
RUN USER=root cargo new --lib sync-client

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY sync-server/Cargo.toml ./sync-server/
COPY sync-core/Cargo.toml ./sync-core/
COPY sync-client/Cargo.toml ./sync-client/

# Build dependencies - this is the caching Docker layer
RUN cargo build --release --bin sync-server
RUN rm -rf sync-server/src sync-core/src sync-client/src

# Copy source code
COPY sync-core/src ./sync-core/src
COPY sync-server/src ./sync-server/src
COPY sync-client/src ./sync-client/src
COPY sync-server/migrations ./sync-server/migrations

# Build application
RUN touch sync-core/src/lib.rs sync-server/src/main.rs sync-client/src/lib.rs
RUN cargo build --release --bin sync-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/sync-server /usr/local/bin/
COPY --from=builder /app/sync-server/migrations /migrations

EXPOSE 8080

CMD ["sync-server"]