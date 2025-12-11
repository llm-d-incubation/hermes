# multi-stage build for hca-probe: UBI9 base with RDMA support
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

# copy workspace files (need whole workspace due to member dependencies)
COPY Cargo.toml Cargo.lock ./
COPY hca-probe ./hca-probe
COPY hermes ./hermes

# build only hca-probe release binary
RUN cargo build --release --manifest-path hca-probe/Cargo.toml

# runtime stage: UBI9 with only RDMA runtime libraries
FROM registry.access.redhat.com/ubi9/ubi:latest

# install only rdma runtime libraries (no dev headers)
RUN dnf install -y \
    libibverbs \
    libibumad \
    librdmacm \
    && dnf clean all

# copy compiled binary from builder (builds to workspace target dir)
COPY --from=builder /build/target/release/hca-probe /usr/local/bin/hca-probe

# rdma device access typically requires root or privileged container
# but we'll default to non-root and let k8s set securityContext if needed
USER 1000

ENTRYPOINT ["/usr/local/bin/hca-probe"]
CMD ["--format", "shell"]
