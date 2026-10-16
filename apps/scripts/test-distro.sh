#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/distro.sh
source "${script_dir}/lib/distro.sh"

assert_eq() {
	local expected="$1" actual="$2"
	if [[ "${expected}" != "${actual}" ]]; then
		printf 'expected %q, got %q\n' "${expected}" "${actual}" >&2
		exit 1
	fi
}

assert_fails() {
	if "$@"; then
		printf 'expected command to fail: %q\n' "$*" >&2
		exit 1
	fi
}

os_release_fixture="$(mktemp)"
trap 'rm -f -- "${os_release_fixture}"' EXIT
export DISTRO_OS_RELEASE_FILE="${os_release_fixture}"

write_os_release() {
	printf '%s\n' "$1" > "${os_release_fixture}"
}

# ── distro_family: exact ID matches ─────────────────────────────────────────
write_os_release 'ID=ubuntu'
assert_eq debian "$(distro_family)"

write_os_release 'ID=debian'
assert_eq debian "$(distro_family)"

write_os_release 'ID=fedora'
assert_eq fedora "$(distro_family)"

write_os_release 'ID=arch'
assert_eq arch "$(distro_family)"

# ── distro_family: ID_LIKE fallback for derivatives ─────────────────────────
printf 'ID=pop\nID_LIKE=ubuntu debian\n' > "${os_release_fixture}"
assert_eq debian "$(distro_family)"

printf 'ID=nobara\nID_LIKE=fedora\n' > "${os_release_fixture}"
assert_eq fedora "$(distro_family)"

printf 'ID=manjaro\nID_LIKE=arch\n' > "${os_release_fixture}"
assert_eq arch "$(distro_family)"

# ── distro_family: unsupported ──────────────────────────────────────────────
write_os_release 'ID=gentoo'
assert_fails distro_family

write_os_release ''
assert_fails distro_family

unset DISTRO_OS_RELEASE_FILE

# ── pkg_name ─────────────────────────────────────────────────────────────────
assert_eq curl "$(pkg_name debian curl)"
assert_eq which "$(pkg_name debian which)"
assert_eq which "$(pkg_name fedora which)"
assert_eq which "$(pkg_name arch which)"
assert_eq fail2ban "$(pkg_name debian fail2ban)"
assert_eq fail2ban "$(pkg_name fedora fail2ban)"
assert_eq fail2ban "$(pkg_name arch fail2ban)"
assert_eq shadow-utils "$(pkg_name fedora uidmap)"
assert_eq '' "$(pkg_name arch uidmap)"
assert_fails pkg_name arch nonexistent

# ── pkg_refresh / pkg_install: dispatch correctness via a capturing fake ────
captured=()
fake_run() {
	captured=("$@")
}

captured=()
pkg_refresh debian fake_run
assert_eq 'apt-get update' "${captured[*]-}"

captured=()
pkg_refresh fedora fake_run
assert_eq 'dnf makecache' "${captured[*]-}"

captured=()
pkg_refresh arch fake_run
assert_eq 'pacman -Sy' "${captured[*]-}"

captured=()
pkg_install debian fake_run curl openssl which fail2ban podman podman_compose uidmap
assert_eq 'apt-get install -y curl openssl which fail2ban podman podman-compose uidmap' "${captured[*]-}"

captured=()
pkg_install fedora fake_run curl openssl which fail2ban podman podman_compose uidmap
assert_eq 'dnf install -y curl openssl which fail2ban podman podman-compose shadow-utils' "${captured[*]-}"

# Arch's uidmap resolves to nothing — must be dropped from the install list,
# not passed through as an empty argv element.
captured=()
pkg_install arch fake_run curl openssl which fail2ban podman podman_compose uidmap
assert_eq 'pacman -S --noconfirm --needed curl openssl which fail2ban podman podman-compose' "${captured[*]-}"

# A request that resolves to nothing at all must not invoke the package
# manager.
captured=()
pkg_install arch fake_run uidmap
assert_eq '' "${captured[*]-}"

printf '%s\n' 'test-distro.sh: all checks passed'
