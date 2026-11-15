# Dev Container Feature: Everruns sandbox guest

Published as `ghcr.io/everruns/features/sandbox-guest`. Add it to an image
you build with the Dev Container CLI, then allow that image on your
deployment (`sandboxes.kubernetes.allowedImages`) and select it in a Sandbox
Template's Isolated sandbox target:

```json
{
  "image": "mcr.microsoft.com/devcontainers/python:3",
  "features": {
    "ghcr.io/everruns/features/sandbox-guest:1": { "version": "v0.53.0" }
  }
}
```

The pod starts `/usr/local/bin/everruns-sandbox-guest` itself, so the
image's entrypoint is not used.
