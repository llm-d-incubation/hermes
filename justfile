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

# run snapshot tests against CoreWeave cluster
test-snapshots-coreweave:
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo test --test snapshot_tests test_coreweave

# run snapshot tests against OpenShift cluster
test-snapshots-openshift:
    cargo test --test snapshot_tests test_openshift

# run all snapshot tests
test-snapshots: test-snapshots-coreweave test-snapshots-openshift

# update snapshots when intentional changes are made
update-snapshots-coreweave:
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo insta test --test snapshot_tests test_coreweave --review

update-snapshots-openshift:
    cargo insta test --test snapshot_tests test_openshift --review

# review all pending snapshot changes
review-snapshots:
    cargo insta review

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

# scan CoreWeave cluster with resource usage stats (forces fresh scan)
scan-coreweave-usage format="table":
    KUBECONFIG="${COREWEAVE_KUBECONFIG}" cargo run -- scan --show-usage --no-cache --format {{format}}

# generate SR-IOV CRDs from OpenShift operator
gen-crds:
    curl -sSL https://raw.githubusercontent.com/openshift/sriov-network-operator/refs/heads/release-4.22/deployment/sriov-network-operator-chart/crds/sriovnetwork.openshift.io_sriovnetworks.yaml | kopium -Af - > src/crds/sriovnetworks.rs
    curl -sSL https://raw.githubusercontent.com/openshift/sriov-network-operator/refs/heads/release-4.22/deployment/sriov-network-operator-chart/crds/sriovnetwork.openshift.io_sriovnetworknodepolicies.yaml | kopium -Af - > src/crds/sriovnetworknodepolicies.rs

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
