# syntax=docker/dockerfile:1
# multi-stage build for hermes + roce-detector: UBI9 base with RDMA support
FROM registry.access.redhat.com/ubi9/ubi:latest AS builder

# install rust toolchain and build dependencies
RUN dnf install -y \
    gcc \
    gcc-c++ \
    make \
    cmake \
    perl-core \
    openssl-devel \
    libibverbs-devel \
    clang-devel \
    rdma-core-devel \
    && dnf clean all

# install rust via rustup
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build

# copy workspace
COPY Cargo.toml Cargo.lock ./
COPY hermes ./hermes
COPY roce-detector ./roce-detector
COPY charts ./charts

# build both binaries
RUN cargo build --release -p hermes && \
    cargo build --release -p roce-detector

# strip binaries
RUN strip target/release/hermes && \
    strip target/release/roce-detector

# export stage for GitHub Actions
FROM scratch AS binaries
COPY --from=builder /build/target/release/hermes /hermes
COPY --from=builder /build/target/release/roce-detector /roce-detector
