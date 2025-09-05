# --- STAGE 1: Build the application ---
# We use a build stage with a full Rust toolchain to compile the application.
FROM rust:latest as builder

# Set the working directory inside the container.
WORKDIR /usr/src/app

# Copy the Cargo.toml and Cargo.lock files first to leverage Docker's layer caching.
# If these don't change, subsequent builds will be faster.
COPY Cargo.toml Cargo.lock ./

# A dummy build to cache dependencies.
RUN mkdir src/
RUN echo "fn main() {}" > src/main.rs
RUN cargo build --release

# Remove the dummy src/main.rs file.
RUN rm -rf src

# Copy the rest of the source code into the container.
COPY . .

# Build the final release binary.
RUN cargo build --release

# --- STAGE 2: Create the final, lightweight image ---
# Use a minimal base image, such as a scratch image or alpine.
# Scratch is the most minimal. You need to statically link to use it.
# If you run into linking errors, use alpine.
FROM debian:stable-slim

# Copy the built binary from the `builder` stage.
# The binary is located at /usr/src/app/target/release/neonote
COPY --from=builder /usr/src/app/target/release/neonote /usr/local/bin/neonote

# Set the command to run the application when the container starts.
CMD ["/usr/local/bin/neonote"]
