**English** · [Español](GOVERNANCE.es.md)

# Governance

This document explains who makes decisions in Litepod and how. It's intentionally simple: this
is a small project, not a company.

## Model

Litepod uses a **single-maintainer (BDFL)** model. Today there is one maintainer who has the
final say on everything. As the project grows, this can change — see [Roles](#roles).

## Roles

### Maintainer

- **Who:** [@Sebasmzt](https://github.com/Sebasmzt) — currently the only maintainer.
- **Powers:** the only person with merge permission. Reviews every pull request. Makes the big
  decisions (direction, breaking changes, releases).
- **Responsibility:** keep the project healthy and honest.

### Contributor

- **Who:** anyone who opens issues or pull requests.
- **Powers:** propose changes, report bugs, request features. No merge permission.

### Future roles

When trusted contributors prove themselves over time, the maintainer may promote them to:

- **Triager** — manage and label issues, help reproduce bugs.
- **Maintainer** — merge pull requests and co-own decisions.

There is no timeline or promise for this. It happens when it happens, based on trust and track
record.

## How decisions are made

- **Small things** (bug fixes, docs, tests, small improvements) are discussed in their issue or
  PR and merged by the maintainer.
- **Big things** (new features, breaking changes, project direction, releases) are decided by
  the maintainer. Contributors propose them as issues first — see
  [`CONTRIBUTING.md`](CONTRIBUTING.md).
- The maintainer can decline or defer anything. Currently the project is in **stability mode**:
  new features are not guaranteed while it stabilizes.

## Reviews

The maintainer reviews every PR. Reviews are done by one person and can take time.

## If the maintainer disappears

Litepod is licensed under **Apache 2.0** (except `ee/`, under the Enterprise License), so the
fallback is simple and always available: **anyone can fork the project and continue it**. No
permission needed.

To make that smoother, the maintainer keeps a private "bus factor" checklist (domain, publishing
credentials, server access) so a trusted person can take over if needed. This checklist is
deliberately not in the public repository.

## Changing this document

Only the maintainer can change this document, by opening a pull request and merging it.
