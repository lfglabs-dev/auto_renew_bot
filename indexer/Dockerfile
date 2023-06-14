# Use the official Rust image as the base image
FROM rust:latest

# Set the working directory
WORKDIR .

# Install protobuf compiler (protoc)
RUN apt-get update && apt-get install -y protobuf-compiler

# Copy the Cargo.toml and Cargo.lock files
COPY indexer/Cargo.toml config.toml ./

# Copy the source code
COPY indexer/src ./src

# Build the application in release mode
RUN cargo build --release

# Expose the port your application uses (replace 8083 with your app's port)
EXPOSE 8080

# Set the unbuffered environment variable
ENV RUST_BACKTRACE "1"

# Run the binary
CMD ["./target/release/indexer"]
