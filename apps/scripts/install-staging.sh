#!/usr/bin/env bash
set -euo pipefail

if [[ "${EUID}" -ne 0 ]]; then
	printf '%s\n' "Run this installer as root: sudo bash apps/scripts/install-staging.sh" >&2
	exit 1
fi

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
apps_dir="$(cd -- "${script_dir}/.." && pwd)"
env_file="${apps_dir}/.env"
env_example="${apps_dir}/.env.example"
compose_file="${apps_dir}/podman-compose-staging.yml"

# shellcheck source=lib/distro.sh
source "${script_dir}/lib/distro.sh"

if [[ "$(uname -s)" != "Linux" ]]; then
	printf '%s\n' "This staging installer requires Linux." >&2
	exit 1
fi

run_direct() {
	"$@"
}

install_packages() {
	local family
	family="$(distro_family)" || {
		printf '%s\n' "Unsupported Linux distribution. Install git, openssl, Podman and podman-compose, then run this script again." >&2
		exit 1
	}
	pkg_refresh "${family}" run_direct
	pkg_install "${family}" run_direct git openssl which podman podman_compose
}

configure_default_registry() {
	local registry_dir="/etc/containers/registries.conf.d"
	local registry_file="${registry_dir}/99-litepod-default-registry.conf"

	install -d -m 0755 "${registry_dir}"
	printf 'unqualified-search-registries = ["docker.io"]\n' > "${registry_file}"
	chmod 0644 "${registry_file}"
}

if ! command -v git >/dev/null 2>&1 || ! command -v openssl >/dev/null 2>&1 || \
   ! command -v podman >/dev/null 2>&1 || ! command -v podman-compose >/dev/null 2>&1; then
	install_packages
fi

configure_default_registry

check_host_resources() {
	local cores mem_kb mem_gib recommended_concurrency
	cores="$(nproc --all 2>/dev/null || echo 1)"
	mem_kb="$(awk '/MemTotal/ {print $2}' /proc/meminfo 2>/dev/null || echo 0)"
	mem_gib=$(( mem_kb / 1024 / 1024 ))

	printf '%s\n' "Host resources: ${cores} CPU core(s), ${mem_gib} GiB RAM."

	if (( cores < 2 || mem_gib < 4 )); then
		recommended_concurrency=1
	elif (( cores < 4 || mem_gib < 8 )); then
		recommended_concurrency=2
	else
		recommended_concurrency=3
	fi

	if (( recommended_concurrency < 3 )); then
		printf '%s\n' "Railpack builds use an isolated rootless BuildKit worker; concurrent builds on" >&2
		printf '%s\n' "a host this size can stall or crash it. After install, set Build Configuration ->" >&2
		printf '%s\n' "Max concurrent builds to ${recommended_concurrency} in Settings (default is 3)." >&2
	fi
}

check_host_resources

configure_rootful_podman() {
	systemctl enable --now podman.socket podman-restart.service
	podman network exists litepod-network || podman network create litepod-network
	podman_mode=rootful
	podman_socket_path=/run/podman/podman.sock
}

configure_rootless_podman() {
	local uid
	uid="$(id -u "${podman_user}")"
	loginctl enable-linger "${podman_user}"
	systemctl start "user@${uid}.service"
	runuser -u "${podman_user}" -- env \
		XDG_RUNTIME_DIR="/run/user/${uid}" \
		DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus" \
		systemctl --user enable --now podman.socket
	runuser -u "${podman_user}" -- env \
		XDG_RUNTIME_DIR="/run/user/${uid}" \
		DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus" \
		systemctl --user enable --now podman-restart.service
	runuser -u "${podman_user}" -- env \
		XDG_RUNTIME_DIR="/run/user/${uid}" \
		DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus" \
		podman network exists litepod-network || \
		runuser -u "${podman_user}" -- env XDG_RUNTIME_DIR="/run/user/${uid}" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus" podman network create litepod-network
	podman_mode=rootless
	podman_socket_path="/run/user/${uid}/podman/podman.sock"
}

podman_user="${LITEPOD_PODMAN_USER:-}"
if [[ -n "${podman_user}" && "${podman_user}" == root ]]; then
	printf '%s\n' "LITEPOD_PODMAN_USER must name an unprivileged user." >&2
	exit 1
fi
if [[ -z "${podman_user}" && -n "${SUDO_USER:-}" && "${SUDO_USER}" != root ]]; then
	podman_user="${SUDO_USER}"
fi
if [[ -n "${podman_user}" ]]; then
	if ! id "${podman_user}" >/dev/null 2>&1; then
		printf '%s\n' "Podman user ${podman_user} does not exist." >&2
		exit 1
	fi
	configure_rootless_podman
else
	configure_rootful_podman
fi

if [[ ! -S "${podman_socket_path}" ]]; then
	printf '%s\n' "Podman's ${podman_mode} socket was not created at ${podman_socket_path}." >&2
	exit 1
fi

podman_socket="unix://${podman_socket_path}"
if [[ "${podman_mode}" == rootless ]]; then
	printf '%s\n' "Podman mode: rootless (user: ${podman_user}, socket: ${podman_socket})"
else
	printf '%s\n' "Podman mode: rootful (socket: ${podman_socket})"
fi

detect_public_ip() {
	local ip
	ip="$(curl -fsS4 --max-time 5 https://api.ipify.org 2>/dev/null)" \
		|| ip="$(curl -fsS4 --max-time 5 https://ifconfig.me 2>/dev/null)" \
		|| ip="$(curl -fsS4 --max-time 5 https://icanhazip.com 2>/dev/null | tr -d '[:space:]')" \
		|| true
	printf '%s' "${ip}"
}

if [[ ! -f "${env_file}" ]]; then
	cp "${env_example}" "${env_file}"
	chmod 600 "${env_file}"

	hash_secret="$(openssl rand -hex 32)"
	encryption_key="$(openssl rand -hex 32)"
	dfly_password="$(openssl rand -hex 32)"
	public_ip="${LITEPOD_PUBLIC_IP:-$(detect_public_ip)}"

	if [[ -z "${public_ip}" ]]; then
		printf '%s\n' "Could not auto-detect the host's public IP (no outbound internet?)." >&2
		printf '%s\n' "Set LITEPOD_PUBLIC_IP in ${env_file}, then rerun this script." >&2
		exit 1
	fi

	# Bootstrap-only value: GitHub App callbacks/webhooks use this until you set a real
	# domain in Settings -> Server Domain after first login, which takes over from then on.
	public_url="http://${public_ip}:80"

	sed -i \
		-e "s#^HASH_SECRET=.*#HASH_SECRET=${hash_secret}#" \
		-e "s#^ENCRYPTION_KEY=.*#ENCRYPTION_KEY=${encryption_key}#" \
		-e "s#^DFLY_PASSWORD=.*#DFLY_PASSWORD=${dfly_password}#" \
		-e "s#^LITEPOD_PUBLIC_IP=.*#LITEPOD_PUBLIC_IP=${public_ip}#" \
		-e "s#^PUBLIC_URL=.*#PUBLIC_URL=${public_url}#" \
		"${env_file}"

	# litepod self-update defaults (read by api/src/updater at boot). The
	# container recreates itself when the image digest at $LITEPOD_IMAGE_REPO:
	# $LITEPOD_IMAGE_TAG changes on the registry. Override with env vars before
	# running this script (e.g. LITEPOD_IMAGE_TAG=staging bash install-staging.sh).
	# Defaults match main.rs: docker.io/litepod/litepod, qa, 3600s.
	grep -q '^LITEPOD_IMAGE_REPO=' "${env_file}" || sed -i "/^PUBLIC_URL=/a LITEPOD_IMAGE_REPO=${LITEPOD_IMAGE_REPO:-docker.io/litepod/litepod}" "${env_file}"
	grep -q '^LITEPOD_IMAGE_TAG=' "${env_file}" || sed -i "/^LITEPOD_IMAGE_REPO=/a LITEPOD_IMAGE_TAG=${LITEPOD_IMAGE_TAG:-qa}" "${env_file}"
	grep -q '^LITEPOD_UPDATE_INTERVAL=' "${env_file}" || sed -i "/^LITEPOD_IMAGE_TAG=/a LITEPOD_UPDATE_INTERVAL=${LITEPOD_UPDATE_INTERVAL:-3600}" "${env_file}"

	printf '%s\n' "Created ${env_file}. Auto-generated JWT key/token, HASH_SECRET, ENCRYPTION_KEY," >&2
	printf '%s\n' "and detected public IP ${public_ip}. Continuing install..." >&2
fi

if ! grep -q '^DFLY_PASSWORD=.' "${env_file}"; then
	dfly_password="$(openssl rand -hex 32)"
	if grep -q '^DFLY_PASSWORD=' "${env_file}"; then
		sed -i "s#^DFLY_PASSWORD=.*#DFLY_PASSWORD=${dfly_password}#" "${env_file}"
	else
		printf '\nDFLY_PASSWORD=%s\n' "${dfly_password}" >> "${env_file}"
	fi
fi

if grep -Eq '^(HASH_SECRET|ENCRYPTION_KEY|LITEPOD_PUBLIC_IP|PUBLIC_URL)=($|replace-with|203\.0\.113\.10|http://your-staging-host)' "${env_file}"; then
	printf '%s\n' "${env_file} still contains missing or placeholder values. Edit it, then rerun." >&2
	exit 1
fi

sed -i '/^LITEPOD_PODMAN_SOCKET=/d; /^LITEPOD_PODMAN_SOCKET_PATH=/d' "${env_file}"
printf 'LITEPOD_PODMAN_SOCKET=%s\nLITEPOD_PODMAN_SOCKET_PATH=%s\n' \
	"${podman_socket}" "${podman_socket_path}" >> "${env_file}"

configure_local_console_ssh() {
	local family console_user console_home ssh_dir authorized_file key_dir public_key host_key fingerprint
	console_user="${podman_user:-${LITEPOD_HOST_USER:-${SUDO_USER:-}}}"
	if [[ -z "${console_user}" || "${console_user}" == root ]] || ! id "${console_user}" >/dev/null 2>&1; then
		printf '%s\n' 'Local console setup requires an existing unprivileged installer user. Set LITEPOD_HOST_USER and rerun.' >&2
		return 1
	fi
	if ! command -v sshd >/dev/null 2>&1; then
		family="$(distro_family)" || return 1
		pkg_refresh "${family}" run_direct
		pkg_install "${family}" run_direct openssh_server
	fi
	ssh-keygen -A
	systemctl enable --now sshd 2>/dev/null || systemctl enable --now ssh
	key_dir=/opt/litepod-staging-host-console
	install -d -m 0700 "${key_dir}"
	[[ -s "${key_dir}/id_ed25519" ]] || ssh-keygen -q -t ed25519 -N '' -C litepod-local-console -f "${key_dir}/id_ed25519"
	chmod 600 "${key_dir}/id_ed25519"
	public_key="$(cat "${key_dir}/id_ed25519.pub")"
	console_home="$(getent passwd "${console_user}" | cut -d: -f6)"
	ssh_dir="${console_home}/.ssh"; authorized_file="${ssh_dir}/authorized_keys"
	install -d -m 0700 -o "${console_user}" -g "$(id -gn "${console_user}")" "${ssh_dir}"
	touch "${authorized_file}"; sed -i '/ litepod-local-console$/d' "${authorized_file}"
	printf 'no-port-forwarding,no-agent-forwarding,no-X11-forwarding,no-user-rc %s\n' "${public_key}" >> "${authorized_file}"
	chown "${console_user}:$(id -gn "${console_user}")" "${authorized_file}"; chmod 0600 "${authorized_file}"
	host_key=/etc/ssh/ssh_host_ed25519_key.pub
	printf 'host.containers.internal %s\n' "$(cat "${host_key}")" > "${key_dir}/known_hosts"
	chmod 644 "${key_dir}/known_hosts"
	fingerprint="$(ssh-keygen -lf "${host_key}" -E sha256 | awk '{print $2}')"
	sed -i '/^LITEPOD_LOCAL_SSH_USER=/d; /^LITEPOD_LOCAL_SSH_FINGERPRINT=/d' "${env_file}"
	printf 'LITEPOD_LOCAL_SSH_USER=%s\nLITEPOD_LOCAL_SSH_FINGERPRINT=%s\n' "${console_user}" "${fingerprint}" >> "${env_file}"
}

configure_local_console_ssh

# Build the Railpack builder image if it doesn't exist.
if ! podman image exists localhost/litepod-railpack-builder:latest; then
	printf '%s\n' "Building Railpack builder image..."
	podman build -f "${apps_dir}/Dockerfile.railpack-builder" -t localhost/litepod-railpack-builder:latest "${apps_dir}"
fi

cd "${apps_dir}"
podman-compose -f "${compose_file}" up -d --build
podman-compose -f "${compose_file}" ps

printf '%s\n' "Staging is running. Verify it with: curl http://localhost/health"
