# Deployment

docray ships as **one container image** containing the server, the worker
CLI, and the pinned PDFium build. The same image runs everywhere.

## Docker

```bash
docker build -t docray .
docker run -d --rm -p 41619:41619 \
  -e DOCRAY_WORKERS=2 \
  docray
```

The image runs as a non-root user with the data directory at `/data`; mount
a volume there if you want job results to survive restarts.

## AWS ECS Fargate

A validated task-definition example lives at
[`deploy/ecs-task-def.example.json`](https://github.com/F2-AI-Inc/docray/blob/main/deploy/ecs-task-def.example.json):
1 vCPU / 5 GB memory with 2 workers (see the
[sizing guidance](configuration.md#sizing-guidance)), a `/healthz` container
health check, and CloudWatch logging. Before registering it:

- push the image to ECR and fill in the image URI,
- create the CloudWatch log group (`/ecs/docray`),
- set an `executionRoleArn` that can pull from ECR and write logs.

## What to know operationally

- **Job state is instance-local** (SQLite + files under `DOCRAY_DATA_DIR`).
  One instance is the supported topology; horizontal scaling would require an
  external job store.
- On restart, jobs that were mid-flight are automatically re-queued.
- The server needs **no outbound network** — only the playground's browser
  assets (pdf.js, fonts) load from CDNs, client-side.
- Responses are not compressed at the HTTP layer yet; if you front docray
  with a reverse proxy, enabling gzip/brotli there shrinks `char`-level
  responses dramatically.
