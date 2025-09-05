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
FROM debian:stable-slim

# Create a non-root user and a working directory
RUN useradd --create-home --shell /bin/bash appuser
WORKDIR /home/appuser/app
USER appuser

# Copy the built binary from the `builder` stage
COPY --from=builder /usr/src/app/target/release/neonote /usr/local/bin/neonote

# Set the command to run the application when the container starts
CMD ["/usr/local/bin/neonote"]
