---
title: Session files hold videos and other large media
description: Session and workspace files stream on download and upload, so recordings play and seek in the browser and files up to 2 GiB fit when object storage is on.
date: 2026-10-26
kind: improvement
significance: minor
products: [platform, cloud]
release: "0.50.1"
surfaces: [api, ui]
links:
  docs: [/sre/environment-variables/]
---

Downloads from session and workspace files now stream from storage and answer HTTP range
requests, so a recorded video plays in the Files viewer and the chat card and can be skipped
around, in every browser. A new `PUT /v1/workspaces/{workspace_id}/fs/_/upload/{path}` takes the
raw file as the request body and streams it to storage, for files larger than the JSON file API
carries.

With object storage on (`STORAGE_BLOB_BACKEND=s3`), a file can be up to 2 GiB and a session up to
10 GiB by default, and copying a file or forking a session no longer copies its stored bytes.
Deployments that keep file bytes in PostgreSQL keep the previous 100 MiB and 500 MiB limits. Both
are set with `SESSION_FILE_SINGLE_MAX_BYTES` and `SESSION_FILE_MAX_BYTES`
([environment variables](/sre/environment-variables/)). Daytona desktop recordings stream from
the Sandbox into session files under the same limit, so recordings over 100 MB are kept too.

```bash
# Upload a recording as the raw request body
curl -X PUT "$EVERRUNS_URL/v1/workspaces/$WORKSPACE_ID/fs/_/upload/recordings/demo.mp4" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H "Content-Type: video/mp4" \
  --data-binary @demo.mp4

# Fetch the first megabyte; the server answers 206 Partial Content
curl "$EVERRUNS_URL/v1/workspaces/$WORKSPACE_ID/fs/_/download/recordings/demo.mp4" \
  -H "Authorization: Bearer $EVERRUNS_API_KEY" \
  -H "Range: bytes=0-1048575" -o first-mb.mp4
```
