# ============================================================================
# Stage 1: Rust builder for roce-detector (uses UBI9 like main Dockerfile)
# ============================================================================
FROM registry.access.redhat.com/ubi9/ubi:latest AS rust-builder

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
    git \
    && dnf clean all

RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build

# copy workspace files for building
COPY Cargo.toml Cargo.lock ./
COPY hermes ./hermes
COPY roce-detector ./roce-detector
COPY charts ./charts

# build roce-detector
RUN cargo build --release -p roce-detector && \
    strip target/release/roce-detector && \
    cp target/release/roce-detector /usr/local/bin/

# ============================================================================
# Stage 2: CUDA perftest builder (from known working version)
# ============================================================================
FROM ubuntu:24.04 AS perftest-builder

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        wget \
        gnupg2 \
        ca-certificates \
        apt-transport-https && \
    rm -rf /var/lib/apt/lists/*

# install CUDA toolkit for building with CUDA support
RUN wget https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2404/x86_64/cuda-keyring_1.1-1_all.deb && \
    dpkg -i cuda-keyring_1.1-1_all.deb && \
    rm cuda-keyring_1.1-1_all.deb && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        cuda-toolkit-12-8 && \
    rm -rf /var/lib/apt/lists/*

ENV PATH=/usr/local/cuda-12.8/bin:${PATH}
ENV LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64:${LD_LIBRARY_PATH}

# install Mellanox OFED dev libraries for building perftest
RUN wget -qO - https://www.mellanox.com/downloads/ofed/RPM-GPG-KEY-Mellanox | gpg --dearmor -o /usr/share/keyrings/mellanox-archive-keyring.gpg && \
    echo "deb [signed-by=/usr/share/keyrings/mellanox-archive-keyring.gpg] https://linux.mellanox.com/public/repo/mlnx_ofed/latest/ubuntu24.04/x86_64 ./" > /etc/apt/sources.list.d/mlnx_ofed.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        libibverbs-dev \
        librdmacm-dev \
        libibumad-dev \
        libpci-dev \
        ibverbs-utils \
        perftest && \
    rm -rf /var/lib/apt/lists/*

# install build tools
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        git \
        build-essential \
        autoconf \
        automake \
        libtool \
        pkg-config && \
    rm -rf /var/lib/apt/lists/*

# build perftest with CUDA support
RUN git clone https://github.com/linux-rdma/perftest.git /tmp/perftest && \
    cd /tmp/perftest && \
    git checkout tags/25.10.0-0.128 && \
    ./autogen.sh && \
    export CUDA_H_PATH=/usr/local/cuda/include/cuda.h && \
    ./configure --prefix=/usr/local/perftest-cuda --enable-cudart && \
    make -j$(nproc) && \
    make install && \
    echo "Perftest binaries built with CUDA support"

# ============================================================================
# Stage 3: Final runtime image with MLNX OFED
# ============================================================================
FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && \
    apt-get install -y --no-install-recommends \
        wget \
        gnupg2 \
        ca-certificates \
        apt-transport-https && \
    rm -rf /var/lib/apt/lists/*

# install CUDA runtime libraries only
RUN wget https://developer.download.nvidia.com/compute/cuda/repos/ubuntu2404/x86_64/cuda-keyring_1.1-1_all.deb && \
    dpkg -i cuda-keyring_1.1-1_all.deb && \
    rm cuda-keyring_1.1-1_all.deb && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        cuda-cudart-12-8 \
        cuda-nvrtc-12-8 && \
    rm -rf /var/lib/apt/lists/* && \
    ln -sf /usr/local/cuda/lib64/libcudart.so.12 /usr/local/cuda/lib64/libcudart.so

ENV PATH=/usr/local/cuda-12.8/bin:/usr/sbin:${PATH}
ENV LD_LIBRARY_PATH=/usr/local/cuda-12.8/lib64:${LD_LIBRARY_PATH}

# install Mellanox OFED runtime packages
RUN wget -qO - https://www.mellanox.com/downloads/ofed/RPM-GPG-KEY-Mellanox | gpg --dearmor -o /usr/share/keyrings/mellanox-archive-keyring.gpg && \
    echo "deb [signed-by=/usr/share/keyrings/mellanox-archive-keyring.gpg] https://linux.mellanox.com/public/repo/mlnx_ofed/latest/ubuntu24.04/x86_64 ./" > /etc/apt/sources.list.d/mlnx_ofed.list && \
    apt-get update && \
    apt-get install -y --no-install-recommends \
        libibverbs1 \
        librdmacm1 \
        libibumad3 \
        libibmad5 \
        libpci3 \
        ibverbs-utils \
        ibverbs-providers \
        rdma-core \
        mlnx-tools \
        mlnx-ofed-kernel-utils \
        mft \
        mstflint \
        infiniband-diags && \
    rm -rf /var/lib/apt/lists/*

# install system utilities
COPY roce-detector/docker/system-packages.txt /tmp/system-packages.txt
RUN apt-get update && \
    apt-get install -y --no-install-recommends $(cat /tmp/system-packages.txt) && \
    rm -rf /var/lib/apt/lists/* /tmp/system-packages.txt

# copy binaries from builder stages
COPY --from=rust-builder /usr/local/bin/roce-detector /usr/local/bin/
COPY --from=perftest-builder /usr/local/perftest-cuda/bin/* /usr/local/bin/

# verify binaries
RUN echo "Verifying binaries..." && \
    roce-detector --help && \
    ls -lh /usr/local/bin/ib_* && \
    ldd /usr/local/bin/ib_write_bw || true

RUN update-pciids

RUN sed -i 's/#force_color_prompt=yes/force_color_prompt=yes/' /root/.bashrc

WORKDIR /root

CMD ["/bin/bash"]
