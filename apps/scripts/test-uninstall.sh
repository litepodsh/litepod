#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=uninstall.sh
source "${script_dir}/uninstall.sh"

assert_eq() {
	local expected="$1" actual="$2"
	[[ "${expected}" == "${actual}" ]] || { printf 'expected %q, got %q\n' "${expected}" "${actual}" >&2; exit 1; }
}

assert_fails() {
	if "$@"; then
		printf 'expected command to fail: %q\n' "$*" >&2
		exit 1
	fi
}

parse_uninstall_args --keep-data
assert_eq true "${keep_data}"
assert_eq purge "${application_policy}"
parse_uninstall_args
assert_eq purge "${application_policy}"
parse_uninstall_args --keep-applications
assert_eq false "${keep_data}"
assert_eq keep "${application_policy}"
parse_uninstall_args --purge-applications
assert_eq purge "${application_policy}"
assert_fails parse_uninstall_args --keep-data --purge-applications
assert_fails parse_uninstall_args --keep-applications --purge-applications

compose_file=/tmp/not-used
fake_core_present=true
podman_user_run() {
	if [[ "$1" == podman-compose ]]; then return 2; fi
	if [[ "$1:$2:$3" == podman:container:exists || "$1:$2:$3" == podman:volume:exists ]]; then
		[[ "${fake_core_present}" == true ]] && return 0 || return 1
	fi
	return 0
}
assert_fails cleanup_core_stack

fake_core_present=false
cleanup_core_stack

declare -a workload_containers=(app sidecar)
declare -a workload_projects=()
declare -a fake_calls=()
fake_workloads_present=true
podman_user_run() {
	fake_calls+=("$*")
	case "$1:$2:$3" in
	podman:container:exists)
		[[ "${fake_workloads_present}" == true ]] && return 0 || return 1
		;;
	podman:inspect:--format)
		[[ "$*" == *'.Mounts'* ]] && printf '%s\n' app-data
		return 0
		;;
	podman:volume:ls) return 0 ;;
	podman:rm:-f) fake_workloads_present=false; return 0 ;;
	podman:volume:rm) return 0 ;;
	*) return 0 ;;
	esac
}
application_policy=keep
assert_eq 0 "$(printf '%s\n' "${#fake_calls[@]}")"
purge_deployed_workloads
case "${fake_calls[*]}" in
*'podman rm -f app'*'podman volume rm -f app-data'*) ;;
*) printf '%s\n' 'purge did not remove discovered application resources' >&2; exit 1 ;;
esac

network_removed=false
podman_user_run() {
	case "$1:$2:$3" in
	podman:network:exists) return 0 ;;
	podman:ps:-a) return 0 ;;
	podman:network:rm) network_removed=true; return 0 ;;
	*) return 0 ;;
	esac
}
remove_empty_litepod_network
assert_eq true "${network_removed}"

parse_uninstall_args --yes
assert_eq true "${assume_yes}"

# Nothing destructive may run until the confirmation succeeds.
install_dir=/tmp/litepod
declare -a workload_containers=(litepod-api my-app)
find_install_dir() { :; }
configure_podman_access() { podman_mode=rootless; podman_user=''; }
discover_deployed_workloads() { :; }
destructive_ran=false
stop_rootless_stack_service() { destructive_ran=true; return 1; }
cleanup_keep_data() { destructive_ran=true; }
confirmation_source="$(mktemp)"
for arguments in '' '--keep-applications' '--keep-data'; do
	for reply in '' 'yes' 'UNINSTALL'; do
		printf '%s\n' "${reply}" > "${confirmation_source}"
		# shellcheck disable=SC2086
		assert_fails main ${arguments}
		assert_eq false "${destructive_ran}"
	done
done
printf 'uninstall\n' > "${confirmation_source}"
assert_fails main
assert_eq true "${destructive_ran}"
destructive_ran=false
main --keep-data >/dev/null 2>&1
assert_eq true "${destructive_ran}"
rm -f -- "${confirmation_source}"

# No terminal: refuses unless --yes was passed.
destructive_ran=false
confirmation_source=/nonexistent/tty
assert_fails main
assert_eq false "${destructive_ran}"
assert_fails main --yes
assert_eq true "${destructive_ran}"

keep_data=false
application_policy=purge
listed="$(describe_uninstall)"
[[ "${listed}" == *my-app* && "${listed}" != *'      litepod-api'* ]] || {
	printf '%s\n' 'purge summary must list applications, not core containers' >&2
	exit 1
}

printf '%s\n' 'uninstall tests passed'
