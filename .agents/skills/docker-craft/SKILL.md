---
name: docker-craft
description: Use Docker best practices whenever a Dockerfile, .dockerignore, docker-compose file, or containerized service needs attention - multi-stage builds, layer-cache ordering, image slimming, non-root security, health checks, volume and network wiring, or debugging a container that won't start. Triggers on base-image choices, secrets in builds, dev-versus-prod image splits, slow builds from cache misses, and decisions about graduating from compose to Kubernetes.
---

# Docker Craft

A Docker image is a deployable artifact and an attack surface at the same time. Every rule below optimizes one of three things: build speed through caching, image size, and runtime safety.

## Dockerfile Best Practices

Docker caches each layer keyed on the instruction plus its inputs; a cache miss invalidates that layer *and every layer after it*. Order instructions from least to most frequently changing so code edits never invalidate dependency installs:

```dockerfile
FROM node:22-slim
WORKDIR /app

# 1. dependencies - change only when the lockfile changes
COPY package.json package-lock.json ./
RUN npm ci

# 2. source - changes every commit, so it goes LAST
COPY src/ ./src/
COPY tsconfig.json ./
RUN npm run build
CMD ["node", "dist/server.js"]
```

- `COPY package*.json ./`, never `COPY . .` at the top - copying everything upfront busts the cache on every edit.
- `.dockerignore` is critical, not optional: without it the build context ships `node_modules`, `.git`, and `.env` (secret leak!) to the daemon on every build, slowing it and baking secrets into layers.

```
# .dockerignore
.git
node_modules
dist
.env
*.log
```

## Multi-Stage Builds

Compilers and build tools belong in a throwaway stage; the final image ships only runtime artifacts. WHY: a Rust builder drags ~2 GB of toolchain; the runtime binary needs none of it.

```dockerfile
FROM rust:1.83 AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
# cache registry deps behind a dummy build, then overlay real source
RUN mkdir src && echo "fn main(){}" > src/main.rs && cargo build --release
COPY src/ ./src/
RUN touch src/main.rs && cargo build --release

FROM debian:bookworm-slim
COPY --from=build /src/target/release/app /usr/local/bin/app
CMD ["app"]
```

Final image size drops from gigabytes to tens of megabytes, and the attack surface shrinks with it.

## Image Size Optimization

- Prefer `alpine` or `-slim` base tags: `node:22-slim` is roughly 75 MB versus ~1 GB for `node:22`.
- `apt-get install --no-install-recommends` skips docs and locales; clean the package cache *in the same layer* or the cleanup only adds another layer:

```dockerfile
RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/*
# single RUN: cache files never exist in any intermediate layer
```

- Strip debug symbols from native binaries: `strip target/release/app`.
- Run `docker history <image>` to see per-layer sizes and find the offender.

## Docker Compose

```yaml
services:
  api:
    build: .
    ports: ["8080:8080"]
    environment:
      DATABASE_URL: postgres://app:${DB_PASSWORD}   # from .env - never inline secrets
    depends_on:
      db:
        condition: service_healthy                  # waits for READY, not just started
    volumes:
      - ./src:/app/src                              # bind mount: live source, dev only
      - app-cache:/app/.cache                       # named volume: managed by compose
  db:
    image: postgres:16
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U app"]
      interval: 5s
      retries: 10
    volumes:
      - pgdata:/var/lib/postgresql/data
volumes:
  pgdata:
  app-cache:
```

- Services reach each other by name on the compose network (`postgres://db:5432`) - no hostfiles or IPs to manage.
- **Volume types**: *named* volumes (docker-managed, portable, for data), *bind* mounts (host directory mapped in, for dev hot-reload), *tmpfs* (in-memory, for secrets and scratch).
- `depends_on` without a healthcheck only orders *container start*, not *service readiness*. The `service_healthy` condition is what prevents the classic "api crash-looped because postgres wasn't ready yet" failure.
- Layer environment variables: image defaults < compose file < `.env` < shell. Put secrets only in the untracked layers.

## Dev vs Production Images

| | Dev | Prod |
|---|---|---|
| Source | bind-mounted, hot reload (`--watch`, nodemon, bacon) | compiled and copied into the image |
| Tools | debuggers, test deps, a shell | nothing beyond the runtime |
| Config | env file, loose defaults | explicit env, fail fast on missing vars |

Build them as targets of the same Dockerfile (`--target dev` / `--target prod`) so they cannot drift apart silently.

## Security

- **Non-root user**: container root is still root, so a container escape lands as root on the host.

```dockerfile
RUN groupadd -r app && useradd -r -g app app
USER app
```

- **Read-only root filesystem** (compose: `read_only: true` plus `tmpfs: ["/tmp"]`) - injected binaries and tampering have nowhere to land.
- **No secrets in images**: build args leak via `docker history`. Pass secrets at runtime as env or files, or use BuildKit `--secret`, which never persists them in a layer.
- Scan in CI: `trivy image myapp:latest` - treat CRITICAL/HIGH findings as blocking.
- Minimal bases (distroless, scratch, alpine) shrink attack surface simply by shipping fewer tools.

## Health Checks

```dockerfile
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
  CMD wget -q -O /dev/null http://localhost:8080/healthz || exit 1
```

Check that the service actually *serves*, not merely that the process is alive - a process can be up while deadlocked or out of connections. `/healthz` should verify dependencies (DB ping, queue reachability) without doing heavy work. Prefer a check built into your binary over installing curl just for the probe; use `wget` or a small script where curl isn't available.

## Debugging Containers

```bash
docker exec -it <container> sh            # shell inside the running container
docker logs -f --tail 100 <container>     # follow output, skip to the last 100 lines
docker inspect <container>                # full metadata: env, mounts, networks, state
docker inspect -f '{{.State.Health}}' <container>   # just the health status
docker network inspect <network>          # attached containers, IPs, aliases
```

If the image has no shell (distroless/scratch), `docker cp` files out, read `docker logs`, or debug a sibling image built with a shell. For connection failures between services, check `docker network inspect` (name resolution, same network?) before blaming the application.

## When to Move from Compose to Kubernetes

Move when you see these signals - not before, because compose on one host is dramatically simpler to operate:

- **Multi-host**: one machine can't hold the workload, or fault isolation demands separate machines.
- **Auto-scaling**: traffic requires adding and removing instances automatically.
- **Rolling updates and self-healing**: you need zero-downtime rollout and automatic restart/rescheduling as platform behavior rather than hand-written scripts.

Until those apply, compose (or Nomad/Swarm as a middle step) is the right tool.

## Common Pitfalls

- `COPY . .` before dependency install - cache busted on every commit; slow builds forever.
- Missing `.dockerignore` - context bloat and `.env` secrets baked into image layers.
- `apt-get clean` in a separate `RUN` - packages still exist in the previous layer; merge the cleanup.
- Running as root "because permissions" - fix ownership instead; root in a container is a real risk.
- Secrets in build args - `docker history` exposes them; rotate and switch to runtime env or BuildKit secrets.
- `depends_on` without health checks - start order mistaken for readiness; cascading crash loops.
- One giant base image for everything - 1 GB images slow every deploy and enlarge attack surface.

## Definition of Done

- [ ] Layers ordered least-to-most volatile; dependency layer survives a code-only change
- [ ] `.dockerignore` present; build context contains no secrets, `.git`, or build output
- [ ] Multi-stage build; final image minimal, runs as non-root, and scans clean (findings triaged)
- [ ] Compose services wired by name, readiness via health checks, volumes correctly typed
- [ ] Dev and prod images built from the same Dockerfile targets
- [ ] Health check verifies real serving; debugging done with exec/logs/inspect, root cause noted
