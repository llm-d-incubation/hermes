set dotenv-load := true

run +args="":
    #!/usr/bin/env bash
    if [[ "{{args}}" == *"coreweave=true"* ]]; then
        KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo run
    elif [[ "{{args}}" == *"gke=true"* ]]; then
        # use current context for GKE (assumes gcloud container clusters get-credentials was run)
        cargo run
    else
        cargo run
    fi

# run nixl self-test on CoreWeave - dry run only (shows manifests)
nixl-self-test-dry:
    #!/usr/bin/env bash
    set -e
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo run -- self-test --dry-run --namespace default 2>/dev/null | \
        awk '/^---$/,0' | grep -v "^======" | grep -v "^------" | grep -v "^✅"

# run nixl self-test on CoreWeave with log streaming
nixl-self-test:
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo run -- self-test --namespace default

# clean up nixl self-test resources
nixl-self-test-cleanup:
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" kubectl delete jobs,configmaps,services -n default -l app=nixl-transfer-test

# run deepep low-latency test on OpenShift with custom topology rule
deepep-openshift:
    HTTPS_PROXY=http://10.2.32.57:3128 cargo run -- self-test --namespace llm-test --workload deepep-lowlatency-test --topology-rule 'string(int(extract(node_name, "r(\\d+)")) / 10)' --gpus-per-node 1 --image ghcr.io/llm-d/llm-d-cuda-dev:latest

# scan CoreWeave cluster with resource usage stats (forces fresh scan)
scan-coreweave-usage format="table":
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo run -- scan --show-usage --no-cache --format {{format}}

# generate SR-IOV CRDs from OpenShift operator
gen-crds:
    curl -sSL https://raw.githubusercontent.com/openshift/sriov-network-operator/refs/heads/release-4.22/deployment/sriov-network-operator-chart/crds/sriovnetwork.openshift.io_sriovnetworks.yaml | kopium -Af - > src/crds/sriovnetworks.rs
    curl -sSL https://raw.githubusercontent.com/openshift/sriov-network-operator/refs/heads/release-4.22/deployment/sriov-network-operator-chart/crds/sriovnetwork.openshift.io_sriovnetworknodepolicies.yaml | kopium -Af - > src/crds/sriovnetworknodepolicies.rs

# generate NVIDIA Network Operator CRD from cluster
gen-nvidia-crd:
    #!/usr/bin/env bash
    set -e
    HTTPS_PROXY=http://10.2.32.57:3128 kubectl get crd nicclusterpolicies.mellanox.com -o yaml > /tmp/nicclusterpolicy-crd.yaml
    kopium -f /tmp/nicclusterpolicy-crd.yaml --derive Default --derive PartialEq > src/crds/nvidia_network.rs
    # fix missing Default derives on enums
    sed -i '' 's/^#\[derive(Serialize, Deserialize, Clone, Debug, PartialEq)\]$$/&#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]/' src/crds/nvidia_network.rs
    # add #[default] to first enum variant for status enums
    sed -i '' '/^pub enum NicClusterPolicyStatus.*State {$/,/^}$/ { /    #\[serde(rename/{ N; s/\n/\n    #[default]\n/; } }' src/crds/nvidia_network.rs
    echo "✅ NVIDIA Network Operator CRD generated at src/crds/nvidia_network.rs"

# bump version (patch, minor, or major) and create release
bump level="patch":
    #!/usr/bin/env bash
    set -e

    # check if cargo-edit is installed
    if ! command -v cargo-set-version &> /dev/null; then
        echo "❌ cargo-edit not installed. Installing..."
        cargo install cargo-edit
    fi

    # get current version
    OLD_VERSION=$(grep -m1 'version = ' Cargo.toml | cut -d'"' -f2)
    echo "Current version: ${OLD_VERSION}"

    # bump version based on level
    cargo set-version --bump {{level}}

    # get new version
    NEW_VERSION=$(grep -m1 'version = ' Cargo.toml | cut -d'"' -f2)
    TAG="v${NEW_VERSION}"

    echo "Bumped version: ${OLD_VERSION} → ${NEW_VERSION}"

    # commit changes
    git add Cargo.toml Cargo.lock
    git commit -m "bump version to ${NEW_VERSION}"

    # create and push tag
    git tag -a "${TAG}" -m "Release ${TAG}"

    echo "✅ Version bumped to ${NEW_VERSION}"
    echo "✅ Changes committed and tagged as ${TAG}"
    echo ""
    echo "Push with: git push && git push origin ${TAG}"

# create git tag from Cargo.toml version
tag-release:
    #!/usr/bin/env bash
    set -e
    VERSION=$(grep -m1 'version = ' Cargo.toml | cut -d'"' -f2)
    TAG="v${VERSION}"
    echo "Creating tag ${TAG}..."
    git tag -a "${TAG}" -m "Release ${TAG}"
    echo "✅ Tag ${TAG} created. Push with: git push origin ${TAG}"

# build hca-probe docker image
build-hca-probe tag="latest":
    #!/usr/bin/env bash
    set -e

    # detect container runtime (prefer podman for RHEL compatibility)
    if command -v podman &> /dev/null; then
        CONTAINER_CMD="podman"
    elif command -v docker &> /dev/null; then
        CONTAINER_CMD="docker"
    else
        echo "Error: neither podman nor docker found"
        exit 1
    fi

    echo "Building hca-probe with ${CONTAINER_CMD}..."
    cd hca-probe
    ${CONTAINER_CMD} build -t quay.io/wseaton/hca-probe:{{tag}} .

    # also tag as latest if a version tag was specified
    if [ "{{tag}}" != "latest" ]; then
        ${CONTAINER_CMD} tag quay.io/wseaton/hca-probe:{{tag}} quay.io/wseaton/hca-probe:latest
    fi

    echo "✅ Image built: quay.io/wseaton/hca-probe:{{tag}}"

# push hca-probe docker image
push-hca-probe tag="latest":
    #!/usr/bin/env bash
    set -e

    # detect container runtime
    if command -v podman &> /dev/null; then
        CONTAINER_CMD="podman"
    elif command -v docker &> /dev/null; then
        CONTAINER_CMD="docker"
    else
        echo "Error: neither podman nor docker found"
        exit 1
    fi

    echo "Pushing hca-probe:{{tag}} to quay.io..."
    ${CONTAINER_CMD} push quay.io/wseaton/hca-probe:{{tag}}

    # if pushing a version tag, also push latest
    if [ "{{tag}}" != "latest" ]; then
        echo "Pushing hca-probe:latest to quay.io..."
        ${CONTAINER_CMD} push quay.io/wseaton/hca-probe:latest
    fi

    echo "✅ Image pushed: quay.io/wseaton/hca-probe:{{tag}}"

# build and push hca-probe docker image
build-push-hca-probe tag="latest": (build-hca-probe tag) (push-hca-probe tag)
