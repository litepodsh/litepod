#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=install.sh
source "${script_dir}/install.sh"

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

wipe_called=false
fake_wipe() {
	wipe_called=true
}

fake_manifest_ok() {
	return 0
}

fake_manifest_fails() {
	return 1
}

assert_eq stable "$(parse_channel)"
assert_eq alpha "$(parse_channel --alpha)"
assert_eq stable "$(parse_channel --wipe)"
assert_eq alpha "$(parse_channel --alpha --wipe)"
assert_eq alpha "$(parse_channel --wipe --alpha)"
has_wipe_flag --wipe
has_wipe_flag --alpha --wipe
assert_fails has_wipe_flag --alpha
can_change_channel stable stable false
can_change_channel stable alpha true
assert_fails can_change_channel stable alpha false
assert_fails parse_channel --unknown
assert_eq 'Usage: bash install.sh [--alpha] [--wipe]' \
	"$(bash -s -- --help < "${script_dir}/install.sh")"
assert_eq 'Usage: bash install.sh [--alpha] [--wipe]' \
	"$(bash "${script_dir}/install.sh" --help)"
should_run_installer_main '' bash
should_run_installer_main scripts/install.sh scripts/install.sh
assert_fails should_run_installer_main scripts/install.sh bash
assert_eq v0.3.27 "${FALLBACK_LITEPOD_IMAGE_TAG}"

manifest_env="$(mktemp)"
trap 'rm -f -- "${manifest_env}"' EXIT
printf '%s\n' 'LITEPOD_VERSION_MANIFEST_URL=https://existing.test/litepod/version.json' > "${manifest_env}"
assert_eq 'https://existing.test/litepod/version.json' \
	"$(LITEPOD_VERSION_MANIFEST_URL= select_manifest_url "${manifest_env}")"
assert_eq 'https://override.test/litepod/version.json' \
	"$(LITEPOD_VERSION_MANIFEST_URL=https://override.test/litepod/version.json select_manifest_url "${manifest_env}")"
assert_eq 'https://litepod.sh/version.json' \
	"$(LITEPOD_VERSION_MANIFEST_URL= select_manifest_url /tmp/litepod-no-such-env)"
assert_fails confirm_channel_wipe stable alpha fake_wipe stdin <<<'no'
assert_eq false "${wipe_called}"

# The explicit test-only source exercises the same stdin fallback used when
# curl | bash has no usable controlling TTY.
confirm_channel_wipe stable alpha fake_wipe stdin <<<$'yes\nWIPE stable TO INSTALL alpha'
assert_eq true "${wipe_called}"

wipe_called=false
assert_fails resolve_then_confirm_channel_wipe stable alpha fake_manifest_fails fake_wipe stdin <<<$'yes\nWIPE stable TO INSTALL alpha'
assert_eq false "${wipe_called}"

resolve_then_confirm_channel_wipe stable alpha fake_manifest_ok fake_wipe stdin <<<$'yes\nWIPE stable TO INSTALL alpha'
assert_eq true "${wipe_called}"

declare -a fake_podman_calls=()
fake_exists_status=1
podman_user_run() {
	fake_podman_calls+=("$*")
	if [[ "$1:$2:$3" == podman:container:exists || "$1:$2:$3" == podman:volume:exists ]]; then
		return "${fake_exists_status}"
	fi
	return 0
}

assert_eq 0 "$(podman_object_is_absent volume missing; printf '%s' "$?")"
fake_exists_status=0
assert_fails podman_object_is_absent volume present
fake_exists_status=125
assert_fails podman_object_is_absent volume unknown

fake_exists_status=1
podman_user_run() {
	fake_podman_calls+=("$*")
	if [[ "$1" == podman-compose ]]; then
		return 2
	fi
	if [[ "$1:$2:$3" == podman:container:exists || "$1:$2:$3" == podman:volume:exists ]]; then
		return 1
	fi
	return 0
}
compose_file=/tmp/not-used
cleanup_compose_stack

podman_user_run() {
	if [[ "$1" == podman-compose ]]; then
		return 2
	fi
	if [[ "$1:$2:$3" == podman:container:exists && "$4" == litepod-api ]]; then
		return 0
	fi
	return 1
}
assert_fails cleanup_compose_stack

fake_workload_container_present=true
fake_workload_volume_present=true
podman_user_run() {
	if [[ "$1:$2" == podman:ps ]]; then
		[[ "${fake_workload_container_present}" == true ]] && printf '%s\n' litepod-workload
		return 0
	fi
	if [[ "$1:$2:$3" == podman:container:exists ]]; then
		[[ "${fake_workload_container_present}" == true ]] && return 0 || return 1
	fi
	if [[ "$1:$2" == podman:inspect ]]; then
		if [[ "$*" == *'.Mounts'* ]]; then
			[[ "${fake_workload_volume_present}" == true ]] && printf '%s\n' litepod-managed-test
		else
			printf '%s\n' '<no value>'
		fi
		return 0
	fi
	if [[ "$1:$2:$3" == podman:volume:ls ]]; then
		[[ "${fake_workload_volume_present}" == true ]] && printf '%s\n' litepod-managed-test
		return 0
	fi
	if [[ "$1:$2:$3" == podman:volume:inspect ]]; then
		printf '%s\n' '<no value>'
		return 0
	fi
	if [[ "$1:$2:$3" == podman:volume:exists ]]; then
		[[ "${fake_workload_volume_present}" == true ]] && return 0 || return 1
	fi
	if [[ "$1:$2:$3" == podman:rm:-f ]]; then
		fake_workload_container_present=false
		return 0
	fi
	if [[ "$1:$2:$3" == podman:volume:rm ]]; then
		fake_workload_volume_present=false
		return 0
	fi
	return 2
}
cleanup_deployed_workloads
assert_eq false "${fake_workload_container_present}"
assert_eq false "${fake_workload_volume_present}"

podman_user_run() {
	if [[ "$1:$2" == podman:ps ]]; then
		printf '%s\n' litepod-workload
		return 0
	fi
	if [[ "$1:$2:$3" == podman:container:exists ]]; then
		return 0
	fi
	if [[ "$1:$2" == podman:inspect ]]; then
		[[ "$*" == *'.Mounts'* ]] || printf '%s\n' '<no value>'
		return 0
	fi
	if [[ "$1:$2:$3" == podman:volume:ls ]]; then
		return 0
	fi
	if [[ "$1:$2:$3" == podman:rm:-f ]]; then
		return 2
	fi
	return 1
}
assert_fails cleanup_deployed_workloads

printf '%s\n' 'install channel tests passed'
