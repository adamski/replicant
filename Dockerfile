# Build stage
FROM rust:1.82 as builder

WORKDIR /app

# Set SQLx to use offline mode (requires .sqlx directory)
ARG SQLX_OFFLINE=true
ENV SQLX_OFFLINE=$SQLX_OFFLINE

# Create empty project structure
RUN USER=root cargo new --bin replicant-server
RUN USER=root cargo new --lib replicant-core
RUN USER=root cargo new --lib replicant-client
RUN USER=root cargo new --lib replicant

# Copy manifests
COPY Cargo.toml Cargo.lock ./
COPY replicant-server/Cargo.toml ./replicant-server/
COPY replicant-core/Cargo.toml ./replicant-core/
COPY replicant-client/Cargo.toml ./replicant-client/
COPY replicant/Cargo.toml ./replicant/

# Build dependencies - this is the caching Docker layer
RUN cargo build --release --bin replicant-server
RUN rm -rf replicant-server/src replicant-core/src replicant-client/src replicant/src

# Copy source code
COPY replicant-core/src ./replicant-core/src
COPY replicant-server/src ./replicant-server/src
COPY replicant-client/src ./replicant-client/src
COPY replicant/src ./replicant/src
COPY replicant-server/migrations ./replicant-server/migrations
COPY replicant-server/.sqlx ./replicant-server/.sqlx

# Build application
RUN touch replicant-core/src/lib.rs replicant-server/src/main.rs replicant-client/src/lib.rs replicant/src/lib.rs
RUN cargo build --release --bin replicant-server

# Runtime stage
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y \
    ca-certificates \
    postgresql-client \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/replicant-server /usr/local/bin/
COPY --from=builder /app/replicant-server/migrations /migrations

EXPOSE 8080

CMD ["replicant-server"]
