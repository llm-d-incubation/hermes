# Common Template Blocks

This directory contains reusable Jinja2 template macros for Kubernetes manifests. These macros help reduce duplication across workload templates.

## Available Templates

### configmap.yaml.j2
ConfigMap template with conditional binaryData/data for dry-run mode.

**Usage:**
```jinja2
{% from "_common/configmap.yaml.j2" import configmap %}
{{ configmap("script-name", "app-label", configmap_files) }}
```

### pod_metadata_env.yaml.j2
Standard pod metadata environment variables (POD_NAME, NODE_NAME, POD_IP).

**Usage:**
```jinja2
{% from "_common/pod_metadata_env.yaml.j2" import pod_metadata_env %}
env:
{{ pod_metadata_env() }}
```

### volumes.yaml.j2
Common volume definitions for ConfigMaps and shared memory.

**Macros:**
- `configmap_volume(name, configmap_name, mount_path)` - volume mount
- `configmap_volume_source(name, configmap_name)` - volume source
- `dshm_volume(size_gi)` - shared memory volume
- `dshm_mount()` - shared memory mount

**Usage:**
```jinja2
{% from "_common/volumes.yaml.j2" import configmap_volume_source, dshm_volume, dshm_mount %}
volumeMounts:
{{ dshm_mount() }}
volumes:
{{ configmap_volume_source("scripts", "my-configmap") }}
{{ dshm_volume(4) }}
```

### sriov.yaml.j2
SR-IOV network annotations.

**Macros:**
- `sriov_annotation_simple()` - basic SR-IOV annotation
- `sriov_annotation_with_interface()` - SR-IOV with interface name

**Usage:**
```jinja2
{% from "_common/sriov.yaml.j2" import sriov_annotation_simple %}
metadata:
  labels:
    app: my-app
{{ sriov_annotation_simple() }}
```

### labels.yaml.j2
Common label patterns.

**Macros:**
- `job_labels(app_label, role)` - standard job labels
- `service_labels(app_label, role=None)` - service labels

**Usage:**
```jinja2
{% from "_common/labels.yaml.j2" import job_labels %}
metadata:
  labels:
{{ job_labels("my-app", "master") | indent(4) }}
```

### resources.yaml.j2
Resource requests/limits patterns.

**Usage:**
```jinja2
{% from "_common/resources.yaml.j2" import resources %}
{{ resources(rdma_qty="1", gpu_qty="2", memory_req="8Gi", memory_lim="16Gi") }}
```

### ucx_env.yaml.j2
UCX/RDMA environment variables.

**Usage:**
```jinja2
{% from "_common/ucx_env.yaml.j2" import ucx_env %}
env:
{{ ucx_env(log_level="debug") }}
```

## Example

See the refactored templates in workload directories for complete examples.

## Notes

- All templates use Jinja2 macro syntax
- Common templates are automatically loaded into the template environment
- Use `{% from "..." import ... %}` to import macros
- Use `| indent(N)` filter to properly align YAML content
