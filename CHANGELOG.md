# Changelog

## [0.2.1] - 2025-10-10

- Internal GKE refactor

## [0.2.0] - 2025-10-10

### Added
- **DeepEP test workloads**: Added `deepep-internode-test` and `deepep-lowlatency-test` for RDMA validation
- **PPLX kernels test**: New `pplx-kernels-test` workload
- **DeepGEMM workload**: Simple GPU+RDMA benchmark using trait system
- `--gpus-per-node` flag: Override GPU requirements per workload
- GPU availability checking: Prevents deployment of unschedulable workloads
- Build-time file embedding: Manifest templates now embedded at compile time
- Image cache awareness: Smarter node selection based on cached container images
- Test author documentation

### Changed
- Migrated to trait-based workload architecture (removed old template system)
- Improved workload rendering and cleanup logic
- Configurable GPU counts in workload templates

### Fixed
- Low latency test GPU configuration
- Pre-commit GitHub Action references

## [0.1.0] - 2024-XX-XX

Initial release of Hermes - Kubernetes cluster analyzer for RDMA-capable GPU infrastructure
