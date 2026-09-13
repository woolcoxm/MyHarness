---
name: git-mastery
description: Use professional Git workflows whenever touching version control - choosing a branching strategy, rewriting history with rebase, hunting regressions with bisect, resolving merge conflicts, recovering lost commits with the reflog, writing conventional commits, or configuring hooks and ignore patterns. Triggers on tasks involving branches, merges, force pushes, detached HEAD states, submodule decisions, changelog generation, or broken shared history that needs careful repair without losing work.
---

# Git Mastery

Git history is both a communication medium and a safety net. Every rule below protects one of those two properties: readable history so humans and changelog tools can follow it, and recoverability so no mistake is fatal.

## Branching Strategies

| Strategy | Shape | When it wins |
|---|---|---|
| Trunk-based | Everyone merges to `main`; branches live <1 day, hidden behind feature flags | Continuous deployment, fast CI, any team that reviews quickly |
| Feature branches | One branch per feature, merged via PR | Teams >~5 people, async review, no deploy-on-merge |
| Git-flow | `main` + `develop` + `release/*` + `hotfix/*` | Scheduled versioned releases, long support windows |

Decide by team size and deployment frequency: deploying many times a day demands trunk-based, because long-lived branches cause merge hell and stale diffs. Shipping quarterly versioned releases makes git-flow's ceremony pay for itself. Most internal teams do best with short-lived feature branches merged to trunk.

## Rebase vs Merge

- **Merge** preserves true history ("a branch happened"). Use it at public integration points - `main` absorbing a PR.
- **Rebase** replays your commits on top of a new base, producing linear history. Use it on your *own unpushed* commits to keep history bisectable.

```bash
git fetch origin
git rebase origin/main           # replay my commits onto the fresh base
git rebase -i origin/main        # squash fixups, drop dead ends, reword messages
git push --force-with-lease      # NEVER bare --force
```

`--force-with-lease` refuses the push if the remote moved since you last fetched - it fails loudly instead of silently deleting a teammate's work. That failure mode is the entire reason the flag exists.

**Golden rule: never rebase a branch other people have checked out.** Rebase rewrites commit hashes; everyone holding old hashes now diverges invisibly. Shared branches get `merge` or `revert`, full stop.

## Finding Regressions: git bisect

Binary search over history: O(log N) tests for N commits.

```bash
git bisect start
git bisect bad HEAD              # current state is broken
git bisect good v2.1.0           # last known-good tag
# git checks out midpoints; at each one:
git bisect good                  # or: git bisect bad
git bisect reset                 # return to where you started
```

Automate the whole search when a command can decide good/bad:

```bash
git bisect start HEAD v2.1.0
git bisect run cargo test --test login   # prints the first bad commit
git bisect reset
```

The failure must be deterministic per commit - flaky tests poison a bisect run, so skip or fix them first.

## Stash and Worktrees

Stash is for minutes-long context switches; worktrees are for parallel work that lasts hours.

```bash
git stash push -m "wip: auth refactor"  # message = you can find it in `stash list`
git stash pop                           # reapply and drop
```

```bash
git worktree add ../proj-hotfix hotfix-404   # second checkout, no branch switching
git worktree remove ../proj-hotfix
```

WHY worktrees exist: switching branches invalidates build artifacts, language-server state, and editor context. A worktree gives each task its own clean simultaneous checkout - run the slow branch's tests while you fix the urgent bug elsewhere.

## Hooks

```bash
# pre-commit - cheapest place to stop bad code; runs before every commit
cargo fmt --check && cargo clippy --all-targets -- -D warnings

# commit-msg - enforce conventional commits (enables changelog automation)
grep -qE '^(feat|fix|docs|refactor|test|chore|perf|ci)(\(.+\))?!?: .+' "$1" \
  || { echo "message must follow conventional commits"; exit 1; }

# pre-push - last gate before your work becomes someone else's baseline
cargo test
```

`.git/hooks/` files are local and never shared. Install them from a checked-in script, or use a manager (`pre-commit`, `husky`, `lefthook`) so the whole team gets identical gates.

## .gitignore Patterns

```gitignore
# global (~/.config/git/ignore) - editor/OS noise across all repos
.DS_Store
*.swp
Thumbs.db

# repo-local - generated artifacts (rebuildable, so never commit them)
target/
node_modules/
__pycache__/
dist/

# negation: ignore *.log but keep crash.log
*.log
!crash.log
# trap: negation cannot rescue a file whose parent DIRECTORY is excluded
```

The point of ignore rules is reviewable diffs. Global ignores cover personal tooling; repo-local ignores cover the language's build output. Never commit files a build can recreate - history is append-only, so a committed `node_modules` bloats the repo forever.

## Submodules vs Subtrees vs Monorepo

- **Submodules** - pin an external repo at an exact commit inside your tree. Wins: precise versioning, upstream stays independent. Costs: every clone needs `--recurse-submodules`, and contributors constantly forget to bump the pin.
- **Subtrees** - merge another repo's content into a directory of yours. Wins: invisible to consumers, no special clone flags. Costs: heavier history, awkward upstream contributions.
- **Monorepo** - one repository containing everything. Wins: atomic cross-project changes, one toolchain, refactors span services in a single commit. Costs: needs per-path CI and sparse checkout to stay fast as it grows.

Default to a monorepo for code you control; reserve submodules for true external dependencies you must version-pin.

## Recovering from Mistakes

| Command | Index | Working tree | Use when |
|---|---|---|---|
| `git reset --soft HEAD~1` | kept | kept | redo message / recombine commits |
| `git reset --mixed HEAD~1` | reset | kept | re-stage selectively (the default) |
| `git reset --hard HEAD~1` | reset | **destroyed** | discard local work permanently |

- **Public branches**: never reset. Use `git revert <sha>` - it appends an inverse commit, so teammates' history stays valid. History others have pulled is immutable in practice.
- **Cherry-pick** applies one commit elsewhere: `git cherry-pick <sha>` for targeted backports to `release/*` without shipping everything on `main`.
- **Reflog is the safety net**: `git reflog` records every HEAD movement for ~90 days. "Lost" commits are almost always recoverable - find the hash, then `git branch rescued <hash>` or `git reset --hard <hash>`.

## Conflict Resolution

Marker sides are named relative to your current checkout:

```
<<<<<<< HEAD            # "ours" - the branch you are ON
timeout: 30
=======
timeout: 5             # "theirs" - the branch being merged in
>>>>>>> feature/login
```

`--ours`/`--theirs` flip meaning during rebase (you sit on the new base while *your* commits are replayed) - the single most common source of wrong resolutions. `git checkout --ours <file>` picks a side wholesale; only do that when you truly know one side is obsolete.

Enable `rerere` (REuse REcorded REsolution) when the same conflict recurs across rebases:

```bash
git config rerere.enabled true   # records each resolution, replays it automatically
```

Resolve by reading *both* intents and writing the correct union - a conflict means both sides changed the same lines, and either side may be right. Then `git add <file>`, `git rebase --continue` (or `git merge --continue`), and run tests before pushing.

## Conventional Commits

```
feat(auth): add PKCE to the login flow
fix(api)!: return 404 instead of 500 for missing users   # ! = breaking change
docs: explain the rollback procedure
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `chore`, `perf`, `ci`. WHY they matter: they are machine-parseable release semantics - `feat` maps to a semver minor bump, `fix` to a patch, `!` or a `BREAKING CHANGE:` footer to a major. Tools like `git-cliff` and `semantic-release` generate changelogs and tags from them, so the changelog stops being a hand-written fiction at release time.

## Common Pitfalls

- `git push --force` on a shared branch - erases teammates' commits. Use `--force-with-lease` on your own branches; `revert` on shared ones.
- Rebasing shared history - instant invisible divergence for everyone else.
- `git reset --hard` with uncommitted changes - the one git operation that destroys uncommitted work. Stash first.
- Resolving conflicts by blindly taking one side - silently deletes the other side's intent.
- `git add .` sweeping in `.env` files - rotate the keys and scrub history; a pre-commit secrets scanner (gitleaks) is cheaper than the cure.
- Committing generated blobs - history is append-only, so the bloat never shrinks without a painful rewrite.

## Definition of Done

- [ ] Branching strategy fits team size and deployment frequency; branches stayed short-lived
- [ ] No shared branch rebased or force-pushed (`--force-with-lease` only on personal branches)
- [ ] History reads cleanly: conventional-commit messages, fixups squashed, no "wip" commits on `main`
- [ ] Bisect identified the first bad commit, or conflicts resolved with both intents considered and tests green
- [ ] Any mistake recovered via reflog/revert/cherry-pick - no work lost, no public history rewritten
- [ ] Hooks and ignore patterns shared with the team; no secrets or generated files committed
