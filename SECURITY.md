# Litepod Security Policy

Security is a top priority for Litepod. We appreciate the help of security researchers and the community in finding and reporting vulnerabilities.

## Supported Versions

Only the latest stable release receives security fixes. Please confirm the issue on the latest release before reporting.

## How to Report a Vulnerability

**Do not open a public issue, pull request or discussion for security problems.**

1.  **Report privately**, using either:
    *   GitHub: [Report a vulnerability](https://github.com/litepodsh/litepod/security/advisories/new) (preferred).
    *   Email: [security@litepod.sh](mailto:security@litepod.sh).
2.  **Provide clear details** so we can understand and reproduce the issue:
    *   A clear description of the vulnerability and the affected component (API, UI, installer, Litepod Cloud).
    *   The Litepod version and how it is installed (self-hosted, rootless or root, Cloud).
    *   Steps to reproduce, with any sample code, screenshots or videos.
    *   The potential impact.
3.  **Do not make the vulnerability public** until we have investigated and released a fix. This protects our users.
4.  **Allow us time:** we aim to acknowledge your report within 3 business days and will keep you informed of our progress. Time to fix depends on complexity and severity. We will credit you in the advisory unless you prefer to stay anonymous.

## Scope

*   The code in this repository, including the Enterprise features under `ee/` directories.
*   The official installer at `https://litepod.sh/install.sh`.
*   Litepod Cloud (`app.litepod.sh`): test only against accounts and servers you own.

Vulnerabilities in third-party dependencies should be reported upstream. Let us know too if Litepod is affected in an exploitable way.

## What We Expect From You

*   Do not access user data or systems beyond what is necessary to demonstrate the vulnerability.
*   Do not perform denial-of-service (DoS) attacks, spamming or social engineering.
*   Do not modify or destroy data that does not belong to you.

## Our Commitment

We are committed to working with you quickly and responsibly to address any legitimate security vulnerability.

Thank you for helping keep Litepod secure for everyone.
