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

# generate SR-IOV CRDs from OpenShift operator
gen-crds:
    curl -sSL https://raw.githubusercontent.com/openshift/sriov-network-operator/refs/heads/release-4.22/deployment/sriov-network-operator-chart/crds/sriovnetwork.openshift.io_sriovnetworks.yaml | kopium -Af - > src/crds/sriovnetworks.rs
    curl -sSL https://raw.githubusercontent.com/openshift/sriov-network-operator/refs/heads/release-4.22/deployment/sriov-network-operator-chart/crds/sriovnetwork.openshift.io_sriovnetworknodepolicies.yaml | kopium -Af - > src/crds/sriovnetworknodepolicies.rs
