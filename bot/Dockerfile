# Use the official Rust image as the base image
FROM rust:latest

# Set the working directory
WORKDIR /app

# Update package index and install protobuf-compiler
RUN apt-get update && apt-get install -y protobuf-compiler && rm -rf /var/lib/apt/lists/*

# Copy the Cargo.toml, Cargo.lock, build.rs, and other configuration files
COPY config.toml bot/Cargo.toml bot/build.rs ./

# Copy the source code and proto directory
COPY bot/src ./src
COPY bot/proto ./proto
COPY bot/src/sales_tax.json ./bot/src/sales_tax.json

# Build the application in release mode
RUN cargo build --release

# Expose the port your application uses
EXPOSE 8080

# Set the unbuffered environment variable
ENV RUST_BACKTRACE "1"

# Run the binary
CMD ["./target/release/bot"]