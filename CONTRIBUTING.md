# Contributing to Litepod

**English** · [Español](CONTRIBUTING.es.md)

Thanks for wanting to help. Litepod is currently in **stability mode**: the priority is
hardening, bug fixes, tests and documentation. New features are welcome as ideas, but they
are **not guaranteed** to be implemented or merged while the project is stabilizing.

The short version: **open an issue first, get it acknowledged, then open a PR.**

## The flow (no exceptions)

1. **Open an issue.** Use the issue template and pick **Bug report** or **Feature request** from
   the *Type* dropdown. Describe the problem, not the solution.
2. **Say if you'll take it on or not.** Comment on the issue stating whether you'll implement
   it yourself:
   ```
   I'll take this one.
   ```
   If you won't implement it, that's fine too — just don't open a PR. Either way, a fix needs
   an issue first. Wait for the maintainer to acknowledge it (a 👍, an assignment, or an
   explicit "go ahead").
3. **Only then open a PR.** A PR that has no prior issue, or whose issue was not acknowledged,
   will not be reviewed. The PR must reference its issue (e.g. `Closes #123`).

This rule exists to avoid wasted work: many ideas are not aligned with the current direction
and get politely declined before you spend hours on them.

## What is (and isn't) merged right now

**Welcome, and likely merged:**

- Bug fixes with a reproduction
- Tests (missing coverage, flaky tests)
- Documentation fixes and translation fixes
- Small, self-contained improvements
- Dependency bumps that fix a real problem (CVE, broken build)

**Welcome as an issue, but not guaranteed while in stability mode:**

- New features
- New integrations, providers or templates
- Refactors or rewrites of working code
- UI redesigns

Feature requests are not rejected — they are tracked and may be scheduled once the project
leaves stability mode. Opening the issue early is how an idea gets on that list.

## Before you submit

- **Test locally first.** Raise the issue, build the stack and reproduce/verify before pushing
  a PR. See [`apps/README.md`](apps/README.md) and [`apps/TESTING.md`](apps/TESTING.md) for how
  to run the stack locally.
- Run the checks for the part you touched:
  ```sh
  cd apps
  just api-test          # API tests (Rust)
  just ui-test           # UI tests (SvelteKit)
  just fmt               # rustfmt + prettier
  ```
- Keep PRs small and focused on one issue. A large PR that mixes a bug fix and a feature is
  harder to review and more likely to be declined.

## PRs

- Fill out the pull request template — it's pre-filled when you open one, and it lives in
  [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md).
- Reference the issue (`Closes #123`).
- Link any related discussion from the issue comments.

The maintainer reviews every PR. Reviews are done by one person and can take time — if a PR
sits for a while, a polite ping on the linked issue is fine.

## Code of conduct

See [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md). In short: no racism, hate or harassment, and
you can write in **English or Spanish, equally**. Harassment, spam and demand-y comments
("when will this be merged??") get you blocked, not merged.

## Security

If you found a vulnerability, **do not open an issue or PR.** Follow
[`SECURITY.md`](SECURITY.md).
