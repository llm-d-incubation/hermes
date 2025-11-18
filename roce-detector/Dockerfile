# multi-stage build for roce-detector: UBI9 base with RDMA support
FROM registry.access.redhat.com/ubi9/ubi:latest AS builder

# install rust toolchain and RDMA build dependencies
RUN dnf install -y \
    gcc \
    gcc-c++ \
    make \
    cmake \
    clang-devel \
    rdma-core-devel \
    libibverbs-devel \
    && dnf clean all

# install rust via rustup (official recommended method)
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build

# copy workspace Cargo.toml first (if exists) for better layer caching
COPY Cargo.toml ./Cargo.toml
COPY src ./src

# build release binary
RUN cargo build --release

# runtime stage: UBI9 with only RDMA runtime libraries
FROM registry.access.redhat.com/ubi9/ubi:latest

# install only rdma runtime libraries (no dev headers)
RUN dnf install -y \
    libibverbs \
    libibumad \
    librdmacm \
    && dnf clean all

# copy compiled binary from builder
COPY --from=builder /build/target/release/roce-detector /usr/local/bin/roce-detector

# rdma device access typically requires root or privileged container
# but we'll default to non-root and let k8s set securityContext if needed
USER 1000

ENTRYPOINT ["/usr/local/bin/roce-detector"]
CMD ["--format", "shell"]
