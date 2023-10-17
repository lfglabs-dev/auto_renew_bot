# Use the official Rust image as the base image
FROM rust:latest

# Set the working directory
WORKDIR .

# Copy the source code
COPY bot/ .

# Build the application in release mode
RUN cargo build --release

# Expose the port your application uses
EXPOSE 8080

# Set the unbuffered environment variable
ENV RUST_BACKTRACE "1"

# Run the binary
CMD ["./target/release/bot"]
