# DeepGEMM Minimal Test

Smoke test that validates DeepGEMM can be imported and basic CUDA/FP8 tensor ops work. Runs a single Job with 1 GPU.

For the full test suite, use `deepgemm-simple-test` instead.

## Usage

```bash
# via hermes
helm hermes install deepgemm-min ./charts/deepgemm-minimal-test \
  --num-nodes 2 --ib-only -n weaton \
  --set namespace=weaton --set testId=t1

# override version
helm hermes install deepgemm-min ./charts/deepgemm-minimal-test \
  --num-nodes 2 --ib-only -n weaton \
  --set namespace=weaton --set testId=t1 \
  --set deepgemm.version=v2.1.1
```

## Configuration

| Parameter | Description | Default |
|-----------|-------------|---------|
| `deepgemm.version` | Git tag/branch/SHA to checkout | `v2.1.1` |
| `image` | Container image | See values.yaml |
| `imagePullSecrets` | Pull secrets list | `[]` |
| `activeDeadlineSeconds` | Job timeout | `180` |

## Cleanup

```bash
helm uninstall deepgemm-min -n weaton
```
