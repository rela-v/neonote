# STAGE 1: Builder
# Use a Rust base image to build the application.
FROM rust:latest as builder

# Set the working directory for our app inside the container.
WORKDIR /usr/src/app

# Copy the dependency files first to leverage Docker's layer caching.
COPY Cargo.toml Cargo.lock ./

# Create a dummy src/main.rs to build dependencies.
RUN mkdir src/ && echo "fn main() {}" > src/main.rs
# Build dependencies. This layer is cached as long as Cargo.toml and Cargo.lock are unchanged.
RUN cargo build --release

# Copy the rest of the source code.
COPY . .

# Build the final release binary.
RUN cargo build --release

# STAGE 2: Runner
# Use a minimal base image for the final container.
FROM debian:bookworm-slim

# Set the working directory for the application as root.
# We no longer create a separate user to make it "permission-proof".
WORKDIR /usr/src/app

# Create the directory for the database.
RUN mkdir -p /usr/src/app/data/notes_db

# Copy the binary from the builder stage.
COPY --from=builder /usr/src/app/target/release/neonote /usr/local/bin/neonote

# Expose the port the application listens on.
EXPOSE 8080

# The command to run the application.
# It will run as root by default.
CMD ["/usr/local/bin/neonote"]

