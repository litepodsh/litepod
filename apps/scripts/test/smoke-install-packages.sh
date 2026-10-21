#!/usr/bin/env bash
# Container smoke test for the package-resolution path in install.sh.
# Run inside a throwaway container as root:
#   podman run --rm -v "$PWD/scripts:/scripts:ro" fedora:latest \
#       bash /scripts/test/smoke-install-packages.sh
# Deliberately narrower than a full install.sh run: no compose stack, no
# Podman socket setup (Podman-in-container needs its own nested setup, which
# is what the sshd integration tests in apps/api/tests cover instead). This
# only proves the distro detection + package manager invocation succeeds and
# leaves `podman`/`podman-compose` on PATH.
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/distro.sh
source "${script_dir}/../lib/distro.sh"

if [[ "${EUID}" -ne 0 ]]; then
	printf '%s\n' "Run as root inside the test container." >&2
	exit 1
fi

run_direct() {
	"$@"
}

family="$(distro_family)" || {
	printf '%s\n' "Unsupported or undetectable Linux distribution." >&2
	exit 1
}
printf 'Detected distro family: %s\n' "${family}"

pkg_refresh "${family}" run_direct
pkg_install "${family}" run_direct curl openssl which podman podman_compose uidmap

command -v podman >/dev/null 2>&1 || {
	printf '%s\n' "podman not found on PATH after install." >&2
	exit 1
}
command -v which >/dev/null 2>&1 || {
	printf '%s\n' "which not found after installation" >&2
	exit 1
}
command -v podman-compose >/dev/null 2>&1 || {
	printf '%s\n' "podman-compose not found on PATH after install." >&2
	exit 1
}

printf '%s\n' "smoke-install-packages.sh: podman and podman-compose installed OK on ${family}"
