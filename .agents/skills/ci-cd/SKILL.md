---
name: ci-cd
description: Use pipeline design discipline whenever a GitHub Actions workflow, deploy script, release process, or slow build needs attention - fast-feedback stage layout, matrix builds, dependency and Docker-layer caching, secrets handling, blue-green/canary/rolling deploys, quality gates, rollback plans, and semver releases generated from conventional commits. Triggers on PR check configuration, flaky or over-long pipelines, deployment-strategy questions, changelog generation, and artifact or registry publishing.
---

# CI/CD

A pipeline is a feedback machine. Its job is to tell you - in minutes, cheaply, and reliably - whether a change is safe to ship. Every decision below optimizes signal per minute.

## Pipeline Design Principles

- Fast feedback targets: **lint/format < 1 min, unit tests < 5 min, build < 10 min**. WHY: developers context-switch after ~10 minutes and the review queue stalls; slow CI trains people to batch giant PRs, which are harder to review and riskier to merge.
- Parallel jobs: split lint, typecheck, unit tests, and build into independent jobs that run simultaneously.
- Fail fast: run the cheapest, most-likely-to-fail checks first; never pay for a 10-minute build before a 20-second lint fails.

## GitHub Actions

```yaml
name: ci
on:
  pull_request:
  push:
    branches: [main]

jobs:
  lint:                            # cheapest gate runs first
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions/cache@v4     # restore keyed by lockfile hash, so the cache
        with:                      # invalidates exactly when dependencies change
          path: ~/.cargo/registry
          key: cargo-${{ hashFiles('**/Cargo.lock') }}
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo fmt --check

  test:
    needs: lint                    # do not pay for tests if lint already failed
    strategy:
      matrix:                      # multi-platform coverage for free
        os: [ubuntu-latest, windows-latest, macos-latest]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - run: cargo test
      - uses: actions/upload-artifact@v4   # move evidence between jobs
        if: always()
        with: {name: reports-${{ matrix.os }}, path: reports/}

  release:
    if: startsWith(github.ref, 'refs/tags/v')
    needs: test
    environment: production        # protection rules: required reviewers and
    runs-on: ubuntu-latest         # secrets scoped to this environment only
    steps:
      - run: ./scripts/deploy.sh
        env:
          DEPLOY_KEY: ${{ secrets.DEPLOY_KEY }}
```

Key mechanics and why they exist:

- `needs:` builds the job DAG - jobs without dependencies run in parallel, dependent jobs are skipped early on failure.
- `actions/cache` keyed on the lockfile hash recompiles nothing until dependencies actually change; keying on branch names misses on every PR.
- `upload-artifact`/`download-artifact` move build output *between jobs*; caching handles run-to-run reuse.
- Secrets live in protected environments, never in the workflow file - the file is in git history forever, and so would be the secret.

## Test Pipeline Stages

| Stage | Runs on | Contents |
|---|---|---|
| Fast gate | every PR push | lint, format, typecheck (<1 min) |
| Full suite | PR + merge to main | unit tests, integration tests |
| Release candidate | tag / release branch | e2e, performance, smoke tests on a prod-like environment |

WHY the split: PRs get the checks that catch most defects cheaply; expensive e2e runs only where a failure blocks a release instead of on every push. Deploys on merge to `main` should be gated on the full suite; release candidates additionally prove the artifact in a production-shaped environment.

## Deployment Strategies

| Strategy | Downtime | Risk control | Cost |
|---|---|---|---|
| Blue-green | none (instant switch) | old version kept warm for instant rollback | 2x infrastructure |
| Canary | none | gradual 1% -> 10% -> 100%, gated on metrics | modest |
| Rolling | near-none (surge instances) | mixed versions serve during rollout | baseline |

Blue-green wins when rollback speed is everything. Canary wins when you do not trust the change and want metrics to decide promotion. Rolling is the cheap default. **Feature flags decouple deploy from release**: ship dark, enable progressively, kill instantly - the flag becomes your real rollback button for behavior, no deploy required.

## Build Optimization

- Docker layer caching in CI: `docker/build-push-action` with `cache-from: type=gha` and `cache-to: type=gha,mode=max`.
- Cache every dependency store keyed on lockfile hash (`Cargo.lock`, `package-lock.json`, `go.sum`).
- Parallel test splitting: shard the suite across matrix jobs (`--partition i/n` or timing-based tools), merge the results.
- Prune the matrix: only run platform combinations you actually ship; testing macOS you never deploy is pure spend.

## Quality Gates

| Gate | Blocking? | Why |
|---|---|---|
| Diff coverage threshold | blocking on PR | catches untested new code; repo-wide % is gameable |
| Security scanning (trivy, codeql) | blocking for CRITICAL/HIGH | unpatched CVEs ship otherwise |
| Dependency audit (`cargo audit`, `npm audit`) | blocking when exploitable | informational for the remainder |
| License check (GPL in a product) | informational first | a legal decision, not a technical one |

Enforce hard gates only where a miss is expensive. Every noisy blocking gate teaches developers to ignore all gates.

## Rollback Strategy

- **Code**: `git revert` the commits and redeploy - forward fixes on `main`, never rewrite public history.
- **Containers**: redeploy the previous image tag; keep the last N tags in the registry so rollback is a tag change measured in minutes.
- **Database migrations - the hard part**: design migrations backward-compatible. Expand (add nullable column), deploy code that writes both, backfill, then contract in a *later* deploy. A rollback that must run `DROP COLUMN` will lose data. Test the rollback path in CI for every migration.
- **Feature flags**: instant behavioral rollback with no deploy at all.

## Artifacts and Releases

```bash
git tag v1.4.2 && git push origin v1.4.2   # semver tag triggers the release job
```

- Semver: breaking = major, feature = minor, fix = patch - the same mechanical rule conventional commits encode.
- Generate the changelog from conventional commits (`git-cliff`, `semantic-release`) so it stops being a chore and stops drifting from what actually shipped.
- Publish binaries and container images to registries (ghcr.io, crates.io, npm) from the tagged workflow, with provenance attestation where the registry supports it.

## Common Pitfalls

- One mega-job that takes 25 minutes - no parallelism; split it and wire `needs`.
- Caches keyed on branch name - miss on every PR; key on lockfile hash instead.
- Secrets committed in workflow YAML - rotate immediately; they are in history forever.
- E2E on every PR push - queue times explode; run on merge or release candidate.
- 100% repo-wide coverage enforced - produces proxy tests and games; enforce diff coverage.
- No rollback rehearsal - the first attempt happens during an incident; test the path.
- Deploying and releasing as one step - you cannot turn off a bad feature without a full rollback.

## Definition of Done

- [ ] Lint < 1 min, tests < 5 min, build < 10 min; jobs parallelized with a sensible `needs` graph
- [ ] Caches keyed on lockfiles; Docker layer cache enabled; artifacts flow between jobs
- [ ] PR / main / release stages run the right checks at the right cadence
- [ ] Secrets only in protected environments; matrix covers only shipped platforms
- [ ] Deployment strategy chosen deliberately; rollback path tested, including migrations
- [ ] Tags are semver; changelog generated from conventional commits; artifacts published
