#!/usr/bin/env bash
set -euo pipefail

usage() {
	printf '%s\n' "Usage: sudo bash apps/scripts/uninstall-staging.sh [--purge-data]"
}

purge_data=false
case "${1:-}" in
	"") ;;
	--purge-data) purge_data=true ;;
	-h|--help) usage; exit 0 ;;
	*) usage >&2; exit 1 ;;
esac

if [[ "${EUID}" -ne 0 ]]; then
	printf '%s\n' "Run this uninstaller as root." >&2
	exit 1
fi

if ! command -v podman-compose >/dev/null 2>&1; then
	printf '%s\n' "podman-compose is not installed; nothing was changed." >&2
	exit 1
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
apps_dir="$(cd -- "${script_dir}/.." && pwd)"
compose_file="${apps_dir}/podman-compose-staging.yml"
env_file="${apps_dir}/.env"

remove_local_console_access() {
	local console_user console_home authorized_file
	console_user="$(sed -n 's/^LITEPOD_LOCAL_SSH_USER=//p' "${env_file}" 2>/dev/null | head -n1)"
	if [[ -n "${console_user}" && "${console_user}" != root ]]; then
		console_home="$(getent passwd "${console_user}" | cut -d: -f6)"
		authorized_file="${console_home}/.ssh/authorized_keys"
		[[ ! -f "${authorized_file}" ]] || sed -i '/ litepod-local-console$/d' "${authorized_file}"
	fi
	rm -f /opt/litepod-staging-host-console/id_ed25519 /opt/litepod-staging-host-console/id_ed25519.pub /opt/litepod-staging-host-console/known_hosts
	rmdir /opt/litepod-staging-host-console 2>/dev/null || true
}

cd "${apps_dir}"
remove_local_console_access
if [[ "${purge_data}" == true ]]; then
	podman-compose -f "${compose_file}" down -v
	printf '%s\n' "Staging containers and named volumes were removed. apps/.env was kept."
else
	podman-compose -f "${compose_file}" down
	printf '%s\n' "Staging containers and networks were removed; LibSQL and Caddy data volumes were kept."
	printf '%s\n' "Use --purge-data to remove the named volumes as well."
fi
