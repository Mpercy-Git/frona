# Frona — Kubernetes Deployment

Kubernetes manifests for deploying Frona with browser automation and web search in a single pod.

## Prerequisites

- A Kubernetes cluster (1.25+)
- `kubectl` configured to access your cluster

## Quick Start

```bash
# 1. Edit the secret — set your encryption secret and at least one LLM API key
#    Edit frona-secret.yaml directly, then:

# 2. Deploy all resources
kubectl apply -k .

# 3. Wait for the pod to be ready
kubectl -n frona get pods -w

# 4. Access Frona (port-forward for local testing)
kubectl -n frona port-forward svc/frona 3001:3001
open http://localhost:3001
```

## Architecture

All three containers run in a single pod and communicate via `localhost`:

| Container | Description |
|---|---|
| **frona** | Frona server (port 3001) |
| **browserless** | Headless Chromium for browser automation |
| **searxng** | Meta search engine for web search |

## Configuration

- **`searxng-configmap.yaml`** — SearXNG search engine configuration
- **`frona-pvc.yaml`** — Storage claim (default 10Gi, adjust as needed)

## Ingress

The Service is `ClusterIP` by default. To expose it externally, add an Ingress:

```yaml
apiVersion: networking.k8s.io/v1
kind: Ingress
metadata:
  name: frona
  namespace: frona
spec:
  rules:
    - host: frona.example.com
      http:
        paths:
          - path: /
            pathType: Prefix
            backend:
              service:
                name: frona
                port:
                  number: 3001
```

## Updating

```bash
kubectl -n frona rollout restart deployment frona
```
