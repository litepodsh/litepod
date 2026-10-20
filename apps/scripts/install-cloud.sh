#!/usr/bin/env bash
# litepod cloud production installer.
#
# Self-contained: does not require a git clone. Pulls the prebuilt image
# (with an immutable tag resolved from litepod's version manifest), starts LibSQL + Caddy + the app
# via Podman Compose.
#
# Two flows, picked automatically:
#   - Run as your normal user (no sudo prefix): sets up ROOTLESS Podman for
#     yourself. This is the default/recommended path. Installs under ~/litepod.
#     Internally calls `sudo` only for the handful of steps that need root
#     (package install, enabling linger, starting your systemd user instance).
#   - Run as root (sudo bash / cloud-init): prompts to choose rootless
#     (default — targets $SUDO_USER, or LITEPOD_PODMAN_USER) or rootful.
#     Installs under /opt/litepod.
#
# Non-interactive root runs (no tty, e.g. piped through `sudo bash` in a
# provisioning script) default to rootless for $SUDO_USER; set LITEPOD_MODE=rootful
# or LITEPOD_MODE=rootless explicitly to skip the prompt.
#
# Usage:
#   curl -fsSL https://litepod.sh/install-cloud.sh | bash            # rootless (default)
#   curl -fsSL https://litepod.sh/install-cloud.sh | sudo bash       # prompts rootless/rootful
#   curl -fsSL https://litepod.sh/install-cloud.sh | sudo LITEPOD_MODE=rootful bash
#   curl -fsSL https://litepod.sh/install-cloud.sh | bash -s -- --alpha
set -euo pipefail

readonly FALLBACK_LITEPOD_IMAGE_TAG="v0.3.6"
readonly FALLBACK_CADDY_IMAGE_TAG="2.11.4"
readonly FALLBACK_DRAGONFLY_IMAGE_TAG="v2.0.2"

use_fallback_image_tags() {
	litepod_image_tag="${FALLBACK_LITEPOD_IMAGE_TAG}"
	caddy_image_tag="${FALLBACK_CADDY_IMAGE_TAG}"
	dragonfly_image_tag="${FALLBACK_DRAGONFLY_IMAGE_TAG}"
}

parse_channel() {
	case "$#:$*" in
	'0:') printf '%s\n' stable ;;
	'1:--alpha') printf '%s\n' alpha ;;
	'1:--help')
		printf '%s\n' 'Usage: bash install.sh [--alpha]'
		return 0
		;;
	*)
		printf '%s\n' 'Usage: bash install.sh [--alpha]' >&2
		return 1
		;;
	esac
}

confirm_channel_wipe() {
	local current_channel="$1" requested_channel="$2" wipe_function="${3:-wipe_channel_installation}" input_source="${4:-}" confirmation expected

	printf '%s\n' 'This permanently deletes the litepod installation, all Podman volumes, database, deployed applications, secrets, domains, and certificates. No backup is made.' >&2
	printf 'Type WIPE %s TO INSTALL %s to continue: ' "${current_channel}" "${requested_channel}" >&2
	expected="WIPE ${current_channel} TO INSTALL ${requested_channel}"
	if [[ "${input_source}" == stdin ]]; then
		IFS= read -r confirmation || confirmation=''
	elif { [[ -r /dev/tty ]] && : < /dev/tty; } 2>/dev/null; then
		if ! IFS= read -r confirmation < /dev/tty; then
			IFS= read -r confirmation || confirmation=''
		fi
	else
		IFS= read -r confirmation || confirmation=''
	fi
	if [[ "${confirmation}" != "${expected}" ]]; then
		printf '%s\n' 'Channel change cancelled; nothing was changed.' >&2
		return 1
	fi

	"${wipe_function}"
}

resolve_then_confirm_channel_wipe() {
	local current_channel="$1" requested_channel="$2" resolve_function="$3" wipe_function="$4" input_source="${5:-}"

	if ! "${resolve_function}"; then
		return 1
	fi
	confirm_channel_wipe "${current_channel}" "${requested_channel}" "${wipe_function}" "${input_source}"
}

should_run_installer_main() {
	local script_source="$1" argv_zero="$2"
	[[ -z "${script_source}" || "${script_source}" == "${argv_zero}" ]]
}

assert_cloud_installation() {
	local target_env_file="$1" mode
	[[ ! -f "${target_env_file}" ]] && return 0
	mode="$(sed -n 's/^IS_CLOUD=//p' "${target_env_file}" | head -n 1)"
	if [[ "${mode}" == true ]]; then
		return 0
	fi
	printf '%s\n' 'This is an existing standard litepod installation. It cannot be converted to IS_CLOUD=true; install cloud on a new host instead.' >&2
	return 1
}

select_manifest_url() {
	local target_env_file="$1" existing=''
	if [[ -f "${target_env_file}" ]]; then
		existing="$(sed -n 's/^LITEPOD_VERSION_MANIFEST_URL=//p' "${target_env_file}" | head -n 1)"
	fi
	printf '%s\n' "${LITEPOD_VERSION_MANIFEST_URL:-${existing:-https://litepod.sh/version.json}}"
}

podman_object_is_absent() {
	local kind="$1" name="$2" status

	if podman_user_run podman "${kind}" exists "${name}"; then
		return 1
	else
		status=$?
	fi
	# `podman <kind> exists` reserves status 1 for a verified absence. Any
	# other failure means we could not prove the destructive cleanup finished.
	[[ "${status}" -eq 1 ]]
}

compose_stack_is_absent() {
	local name
    for name in litepod-caddy litepod-dragonfly litepod-api; do
		podman_object_is_absent container "${name}" || return 1
	done
    for name in litepod_caddy_data litepod_caddy_config; do
		podman_object_is_absent volume "${name}" || return 1
	done
}

cleanup_compose_stack() {
	if podman_user_run podman-compose -f "${compose_file}" down -v --remove-orphans; then
		return 0
	fi
	if compose_stack_is_absent; then
		printf '%s\n' 'Compose cleanup reported an error, but all managed containers and volumes are verified absent.' >&2
		return 0
	fi
	printf '%s\n' 'Compose cleanup failed and managed resources may remain; refusing to delete the installation directory.' >&2
	return 1
}

remove_litepod_data_volume() {
	if podman_user_run podman volume rm -f litepod_data; then
		return 0
	fi
	if podman_object_is_absent volume litepod_data; then
		printf '%s\n' 'litepod_data was already absent.' >&2
		return 0
	fi
	printf '%s\n' 'Could not remove or verify absence of litepod_data; refusing to delete the installation directory.' >&2
	return 1
}

cleanup_deployed_workloads() {
	local seeds members mounts all_volumes project name volume remaining
	local -a containers=() volumes=()

	if ! seeds="$(podman_user_run podman ps -a --filter network=litepod-network --format '{{.Names}}')"; then
		printf '%s\n' 'Could not list litepod workload containers; refusing to continue the wipe.' >&2
		return 1
	fi
	while IFS= read -r name; do
		[[ -n "${name}" ]] && containers+=("${name}")
	done <<< "${seeds}"

	# Compose applications can have private sidecars that are not attached to
	# litepod-network. Expand every discovered public service to its whole
	# Compose project before removing anything.
	for name in "${containers[@]}"; do
		for label in com.docker.compose.project io.podman.compose.project; do
			if ! project="$(podman_user_run podman inspect --format "{{with .Config.Labels}}{{index . \"${label}\"}}{{end}}" "${name}")"; then
				printf '%s\n' "Could not inspect workload container ${name}; refusing to continue the wipe." >&2
				return 1
			fi
			[[ -z "${project}" || "${project}" == '<no value>' ]] && continue
			if ! members="$(podman_user_run podman ps -a --filter "label=${label}=${project}" --format '{{.Names}}')"; then
				printf '%s\n' "Could not list Compose project ${project}; refusing to continue the wipe." >&2
				return 1
			fi
			while IFS= read -r member; do
				[[ -n "${member}" ]] && containers+=("${member}")
			done <<< "${members}"
		done
	done

	# Capture named volumes before deleting their containers.
	for name in "${containers[@]}"; do
		if podman_object_is_absent container "${name}"; then
			continue
		fi
		if ! mounts="$(podman_user_run podman inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{println .Name}}{{end}}{{end}}' "${name}")"; then
			printf '%s\n' "Could not inspect volumes for workload container ${name}; refusing to continue the wipe." >&2
			return 1
		fi
		while IFS= read -r volume; do
			[[ -n "${volume}" ]] && volumes+=("${volume}")
		done <<< "${mounts}"
	done

	# Include detached litepod managed volumes and orphaned application Compose
	# volumes. Application Compose projects always use the `*-compose` suffix.
	if ! all_volumes="$(podman_user_run podman volume ls --format '{{.Name}}')"; then
		printf '%s\n' 'Could not list litepod workload volumes; refusing to continue the wipe.' >&2
		return 1
	fi
	while IFS= read -r volume; do
		[[ -z "${volume}" ]] && continue
		if [[ "${volume}" == litepod-managed-* ]]; then
			volumes+=("${volume}")
			continue
		fi
		project=''
		for label in com.docker.compose.project io.podman.compose.project; do
			if ! project="$(podman_user_run podman volume inspect --format "{{with .Labels}}{{index . \"${label}\"}}{{end}}" "${volume}")"; then
				printf '%s\n' "Could not inspect volume ${volume}; refusing to continue the wipe." >&2
				return 1
			fi
			[[ -n "${project}" && "${project}" != '<no value>' ]] && break
		done
		[[ "${project}" == *-compose ]] && volumes+=("${volume}")
	done <<< "${all_volumes}"

	for name in "${containers[@]}"; do
		if podman_user_run podman rm -f -v "${name}"; then
			continue
		fi
		if ! podman_object_is_absent container "${name}"; then
			printf '%s\n' "Could not remove workload container ${name}; refusing to continue the wipe." >&2
			return 1
		fi
	done
	for volume in "${volumes[@]}"; do
		if podman_user_run podman volume rm -f "${volume}"; then
			continue
		fi
		if ! podman_object_is_absent volume "${volume}"; then
			printf '%s\n' "Could not remove workload volume ${volume}; refusing to continue the wipe." >&2
			return 1
		fi
	done

	if ! remaining="$(podman_user_run podman ps -a --filter network=litepod-network --format '{{.Names}}')"; then
		printf '%s\n' 'Could not verify workload container cleanup; refusing to continue the wipe.' >&2
		return 1
	fi
	if [[ -n "${remaining}" ]]; then
		printf '%s\n' "Workload containers remain after cleanup: ${remaining}" >&2
		return 1
	fi
	if ! all_volumes="$(podman_user_run podman volume ls --format '{{.Name}}')"; then
		printf '%s\n' 'Could not verify workload volume cleanup; refusing to continue the wipe.' >&2
		return 1
	fi
	while IFS= read -r volume; do
		[[ -z "${volume}" ]] && continue
		if [[ "${volume}" == litepod-managed-* ]]; then
			printf '%s\n' "litepod managed volume remains after cleanup: ${volume}" >&2
			return 1
		fi
		project=''
		for label in com.docker.compose.project io.podman.compose.project; do
			if ! project="$(podman_user_run podman volume inspect --format "{{with .Labels}}{{index . \"${label}\"}}{{end}}" "${volume}")"; then
				printf '%s\n' "Could not verify volume ${volume}; refusing to continue the wipe." >&2
				return 1
			fi
			[[ -n "${project}" && "${project}" != '<no value>' ]] && break
		done
		if [[ "${project}" == *-compose ]]; then
			printf '%s\n' "litepod Compose volume remains after cleanup: ${volume}" >&2
			return 1
		fi
	done <<< "${all_volumes}"
}

main() {
if [[ "$#" -eq 1 && "$1" == --help ]]; then
	parse_channel --help
	return 0
fi

if [[ "$(uname -s)" != "Linux" ]]; then
	printf '%s\n' "This installer requires Linux." >&2
	exit 1
fi

# Arrow-key menu, pure bash (no dialog/whiptail/fzf needed). Prints "$@" as a
# list, Up/Down to move, Enter to confirm; sets $menu_selected to the chosen
# string. Reads/writes /dev/tty directly since stdin is the piped script.
menu_select() {
	local -a options=("$@")
	local n=${#options[@]} selected=0 i key rest

	draw() {
		for ((i = 0; i < n; i++)); do
			printf '\e[2K' > /dev/tty
			if [[ ${i} -eq ${selected} ]]; then
				printf '  \e[7m %s \e[0m\n' "${options[$i]}" > /dev/tty
			else
				printf '    %s\n' "${options[$i]}" > /dev/tty
			fi
		done
	}

	draw
	while true; do
		IFS= read -rsn1 key < /dev/tty
		if [[ "${key}" == $'\x1b' ]]; then
			IFS= read -rsn2 -t 0.1 rest < /dev/tty || true
			case "${rest}" in
			'[A') (( selected = (selected - 1 + n) % n )) ;;
			'[B') (( selected = (selected + 1) % n )) ;;
			esac
		elif [[ -z "${key}" ]]; then
			break
		fi
		printf '\e[%dA' "${n}" > /dev/tty
		draw
	done
	menu_selected="${options[$selected]}"
}

# ── Mode selection ──────────────────────────────────────────────────────────
# podman_mode: rootless | rootful
# podman_user: only set for rootless (the user Podman runs as)
# as_root:     true if this script itself is running as EUID 0

requested_channel="$(parse_channel "$@")"

if [[ "${EUID}" -eq 0 ]]; then
	as_root=true

	mode="${LITEPOD_MODE:-}"
	podman_user="${LITEPOD_PODMAN_USER:-${SUDO_USER:-}}"
	[[ "${podman_user}" == root ]] && podman_user=""

	if [[ -z "${mode}" && -r /dev/tty ]]; then
		default_user="${podman_user:-$(logname 2>/dev/null || true)}"
		printf '%s\n' "Install Podman as:" > /dev/tty
		menu_select "rootless (recommended)" "rootful"
		printf '\n' > /dev/tty
		if [[ "${menu_selected}" == rootful ]]; then
			mode=rootful
		else
			mode=rootless
			if [[ -z "${podman_user}" ]]; then
				read -r -p "Podman user [${default_user}]: " typed_user < /dev/tty || true
				podman_user="${typed_user:-${default_user}}"
			fi
		fi
	fi

	mode="${mode:-rootless}"
	if [[ "${mode}" == rootless && -z "${podman_user}" ]]; then
		printf '%s\n' "No target user for rootless Podman (no \$SUDO_USER, no tty to ask)." >&2
		printf '%s\n' "Set LITEPOD_PODMAN_USER=<user>, or LITEPOD_MODE=rootful for rootful Podman." >&2
		exit 1
	fi
	if [[ "${mode}" == rootless ]] && ! id "${podman_user}" >/dev/null 2>&1; then
		printf '%s\n' "Podman user ${podman_user} does not exist." >&2
		exit 1
	fi
	podman_mode="${mode}"
	install_dir="${LITEPOD_INSTALL_DIR:-/opt/litepod}"
else
	as_root=false
	podman_mode=rootless
	podman_user="$(id -un)"
	install_dir="${LITEPOD_INSTALL_DIR:-${HOME}/litepod}"
fi

env_file="${install_dir}/.env"
compose_file="${install_dir}/podman-compose-prod.yml"
assert_cloud_installation "${env_file}"

# Runs a command as root: directly if we're already root, via sudo otherwise.
as_root_run() {
	if [[ "${as_root}" == true ]]; then
		"$@"
	else
		sudo "$@"
	fi
}

# Runs a Podman/Compose command as podman_user with its rootless env, or
# directly when this script is already running as that user.
podman_user_run() {
	if [[ "${as_root}" == true && "${podman_mode}" == rootless ]]; then
		local uid
		uid="$(id -u "${podman_user}")"
		runuser -u "${podman_user}" -- env \
			XDG_RUNTIME_DIR="/run/user/${uid}" \
			DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus" \
			"$@"
	else
		"$@"
	fi
}

wipe_channel_installation() {
	if [[ "${install_dir}" != /opt/litepod && "${install_dir}" != */litepod ]]; then
		printf '%s\n' "Refusing to delete unexpected installation directory: ${install_dir}" >&2
		return 1
	fi

	cleanup_compose_stack
	cleanup_deployed_workloads
	remove_litepod_data_volume
	if [[ "${podman_mode}" == rootless ]]; then
		podman_user_run systemctl --user disable --now litepod-stack.service || true
	fi
	rm -rf -- "${install_dir}"
}

# Rootless Podman publishes 80/443 through rootlessport, which hides the visitor's
# address: Caddy sees the bridge IP, so analytics (Umami) can't resolve countries.
# A systemd socket owns 80/443 on the host and hands them to Caddy as fd/3 and
# fd/4, so Caddy sees the real client address and still sits on litepod-network.
caddy_socket_activation_supported() {
	local version
	[[ "${podman_mode}" == rootless ]] || return 1
	version="$(podman_user_run podman version --format '{{.Client.Version}}' 2>/dev/null || true)"
	[[ "${version}" =~ ^([0-9]+)\.([0-9]+) ]] || return 1
	(( BASH_REMATCH[1] > 4 || (BASH_REMATCH[1] == 4 && BASH_REMATCH[2] >= 4) )) || return 1
	# Quadlet is a podman-user-generator for systemd; older packages ship without it.
	[[ -n "$(find /usr/lib /usr/libexec /lib -maxdepth 4 -name podman-user-generator 2>/dev/null | head -n 1)" ]]
}

# Writes stdin to a file owned by the rootless user.
install_user_file() {
	local dest="$1" tmp
	tmp="$(mktemp)"
	cat > "${tmp}"
	as_root_run chown "${podman_user}:${podman_user}" "${tmp}"
	podman_user_run install -d -m 0755 "$(dirname "${dest}")"
	podman_user_run install -m 0644 "${tmp}" "${dest}"
	rm -f "${tmp}"
}

configure_caddy_socket_activation() {
	local user_home quadlet_dir unit_dir config_dir autosave

	user_home="$(getent passwd "${podman_user}" | cut -d: -f6)"
	if [[ -z "${user_home}" ]]; then
		printf '%s\n' "Could not determine home directory for rootless Podman user ${podman_user}." >&2
		return 1
	fi
	quadlet_dir="${user_home}/.config/containers/systemd"
	unit_dir="${user_home}/.config/systemd/user"

	install_user_file "${quadlet_dir}/litepod-caddy.container" <<EOF
[Unit]
Description=litepod Caddy
Requires=litepod-caddy.socket
After=litepod-caddy.socket

[Container]
Image=docker.io/library/caddy:${caddy_image_tag}
ContainerName=litepod-caddy
Network=litepod-network
Exec=caddy run --resume --config /etc/caddy/Caddyfile --adapter caddyfile
Volume=${install_dir}/Caddyfile:/etc/caddy/Caddyfile:ro
Volume=litepod_caddy_data:/data
Volume=litepod_caddy_config:/config

[Service]
Restart=always

[Install]
WantedBy=default.target
EOF
	# Listener order fixes the descriptors: fd/3 HTTP, fd/4 HTTPS, fdgram/5 HTTP/3 (UDP).
	install_user_file "${unit_dir}/litepod-caddy.socket" <<EOF
[Unit]
Description=litepod Caddy ports

[Socket]
ListenStream=80
ListenStream=443
ListenDatagram=443

[Install]
WantedBy=sockets.target
EOF
	podman_user_run systemctl --user daemon-reload

	# A Compose-managed Caddy holds 80/443 through rootlessport, and its saved config
	# listens on :80/:443 instead of the inherited sockets. Compose may already have
	# removed it as an orphan, so the config is rewritten whether or not it is found.
	podman_user_run podman stop litepod-caddy >/dev/null 2>&1 || true
	config_dir="$(podman_user_run podman volume inspect litepod_caddy_config --format '{{.Mountpoint}}' 2>/dev/null || true)"
	autosave="${config_dir}/caddy/autosave.json"
	if [[ -n "${config_dir}" ]] && podman_user_run grep -q -E '"listen":\[(":80"|"fd/3","fd/4"\])' "${autosave}" 2>/dev/null; then
		podman_user_run cp -p "${autosave}" "${autosave}.pre-fd"
		# Caddy refuses HTTP/3 on a TCP fd listener, so each listener names its protocols.
		# ":443" stays last: API releases that predate inherited sockets re-add it to
		# `listen`, and Caddy rejects the config when listen_protocols has fewer entries.
		podman_user_run sed -i 's/"listen":\[":80",":443"\]/"listen":["fd\/3","fd\/4","fdgram\/5",":443"],"listen_protocols":[["h1","h2"],["h1","h2"],["h3"],["h1","h2"]]/; s/"listen":\[":80"\]/"listen":["fd\/3"],"listen_protocols":[["h1","h2"]]/; s/"listen":\["fd\/3","fd\/4"\]/"listen":["fd\/3","fd\/4","fdgram\/5",":443"],"listen_protocols":[["h1","h2"],["h1","h2"],["h3"],["h1","h2"]]/' "${autosave}"
	fi
	# Drop the server-wide protocols override older revisions of this installer added.
	if [[ -n "${config_dir}" ]]; then
		podman_user_run sed -i 's/"srv0":{"protocols":\["h1","h2"\],/"srv0":{/' "${autosave}" 2>/dev/null || true
	fi
	podman_user_run podman rm litepod-caddy >/dev/null 2>&1 || true
	# A running socket keeps its old listeners; stop both so changed ones bind again.
	podman_user_run systemctl --user stop litepod-caddy.service litepod-caddy.socket
	podman_user_run systemctl --user enable --now litepod-caddy.socket
	podman_user_run systemctl --user start litepod-caddy.service
}

configure_rootless_stack_service() {
	local user_home podman_bin unit_dir unit_file unit_tmp start_list stop_list

	user_home="$(getent passwd "${podman_user}" | cut -d: -f6)"
	if [[ -z "${user_home}" ]]; then
		printf '%s\n' "Could not determine home directory for rootless Podman user ${podman_user}." >&2
		return 1
	fi
	podman_bin="$(command -v podman)"
	if [[ -z "${podman_bin}" ]]; then
		printf '%s\n' "podman is unavailable after installation." >&2
		return 1
	fi
	unit_dir="${user_home}/.config/systemd/user"
	unit_file="${unit_dir}/litepod-stack.service"
	if [[ "${use_caddy_sockets:-false}" == true ]]; then
		start_list="litepod-dragonfly litepod-api"
		stop_list="litepod-api litepod-dragonfly"
	else
		start_list="litepod-dragonfly litepod-caddy litepod-api"
		stop_list="litepod-api litepod-caddy litepod-dragonfly"
	fi
	unit_tmp="$(mktemp)"
	cat > "${unit_tmp}" <<EOF
[Unit]
Description=litepod containers
Wants=network-online.target podman.socket
After=network-online.target podman.socket

[Service]
Type=oneshot
RemainAfterExit=yes
WorkingDirectory=${install_dir}
# Start the installed containers; Compose up can replace UI-updated images
# with older tags and configuration hashes from the installation files.
ExecStart=${podman_bin} start ${start_list}
ExecStop=${podman_bin} stop ${stop_list}
TimeoutStartSec=0
TimeoutStopSec=120

[Install]
WantedBy=default.target
EOF
	as_root_run chown "${podman_user}:${podman_user}" "${unit_tmp}"
	podman_user_run install -d -m 0755 "${unit_dir}"
	podman_user_run install -m 0644 "${unit_tmp}" "${unit_file}"
	rm -f "${unit_tmp}"
	podman_user_run systemctl --user daemon-reload
	podman_user_run systemctl --user enable --now litepod-stack.service
	podman_user_run systemctl --user is-enabled --quiet litepod-stack.service
}

# BEGIN distro.sh sync — keep byte-for-byte identical to scripts/lib/distro.sh
# and scripts/install.sh's copy. This script ships standalone via
# `curl | bash` and can't source a sibling file.
distro_family() {
	local os_release_file="${DISTRO_OS_RELEASE_FILE:-/etc/os-release}" id='' id_like='' token
	if [[ -r "${os_release_file}" ]]; then
		id="$(sed -n 's/^ID=//p' "${os_release_file}" | head -n1 | tr -d '"')"
		id_like="$(sed -n 's/^ID_LIKE=//p' "${os_release_file}" | head -n1 | tr -d '"')"
	fi
	case "${id}" in
	debian | ubuntu)
		printf '%s\n' debian
		return 0
		;;
	fedora)
		printf '%s\n' fedora
		return 0
		;;
	arch)
		printf '%s\n' arch
		return 0
		;;
	esac
	for token in ${id_like}; do
		case "${token}" in
		debian | ubuntu)
			printf '%s\n' debian
			return 0
			;;
		fedora | rhel)
			printf '%s\n' fedora
			return 0
			;;
		arch)
			printf '%s\n' arch
			return 0
			;;
		esac
	done
	return 1
}

pkg_name() {
	local family="$1" logical="$2"
	case "${family}:${logical}" in
	debian:curl) printf '%s\n' curl ;;
	debian:openssl) printf '%s\n' openssl ;;
	debian:git) printf '%s\n' git ;;
	debian:podman) printf '%s\n' podman ;;
	debian:podman_compose) printf '%s\n' podman-compose ;;
	debian:uidmap) printf '%s\n' uidmap ;;
	debian:openssh_server) printf '%s\n' openssh-server ;;
	debian:which) printf '%s\n' which ;;
	debian:fail2ban) printf '%s\n' fail2ban ;;
	fedora:curl) printf '%s\n' curl ;;
	fedora:openssl) printf '%s\n' openssl ;;
	fedora:git) printf '%s\n' git ;;
	fedora:podman) printf '%s\n' podman ;;
	fedora:podman_compose) printf '%s\n' podman-compose ;;
	fedora:uidmap) printf '%s\n' shadow-utils ;;
	fedora:openssh_server) printf '%s\n' openssh-server ;;
	fedora:which) printf '%s\n' which ;;
	fedora:fail2ban) printf '%s\n' fail2ban ;;
	arch:curl) printf '%s\n' curl ;;
	arch:openssl) printf '%s\n' openssl ;;
	arch:git) printf '%s\n' git ;;
	arch:podman) printf '%s\n' podman ;;
	arch:podman_compose) printf '%s\n' podman-compose ;;
	arch:uidmap) : ;; # newuidmap/newgidmap ship in Arch's base `shadow` package already
	arch:openssh_server) printf '%s\n' openssh ;;
	arch:which) printf '%s\n' which ;;
	arch:fail2ban) printf '%s\n' fail2ban ;;
	*)
		printf 'pkg_name: unknown package %s for %s\n' "${logical}" "${family}" >&2
		return 1
		;;
	esac
}

pkg_refresh() {
	local family="$1" run="$2"
	case "${family}" in
	debian) "${run}" apt-get update ;;
	fedora) "${run}" dnf makecache ;;
	arch) "${run}" pacman -Sy ;;
	*)
		printf 'pkg_refresh: unsupported family %s\n' "${family}" >&2
		return 1
		;;
	esac
}

pkg_install() {
	local family="$1" run="$2"
	shift 2
	local -a resolved=()
	local logical name
	for logical in "$@"; do
		name="$(pkg_name "${family}" "${logical}")" || return 1
		[[ -n "${name}" ]] && resolved+=("${name}")
	done
	[[ "${#resolved[@]}" -eq 0 ]] && return 0
	case "${family}" in
	debian) "${run}" apt-get install -y "${resolved[@]}" ;;
	fedora) "${run}" dnf install -y "${resolved[@]}" ;;
	arch) "${run}" pacman -S --noconfirm --needed "${resolved[@]}" ;;
	*)
		printf 'pkg_install: unsupported family %s\n' "${family}" >&2
		return 1
		;;
	esac
}
# END distro.sh sync

install_packages() {
	local family
	family="$(distro_family)" || {
		printf '%s\n' "Unsupported Linux distribution. Install curl, openssl, Podman and podman-compose, then run this script again." >&2
		exit 1
	}
	pkg_refresh "${family}" as_root_run
	pkg_install "${family}" as_root_run curl openssl which podman podman_compose uidmap
}

configure_default_registry() {
	local registry_dir="/etc/containers/registries.conf.d"
	local registry_file="${registry_dir}/99-litepod-default-registry.conf"

	as_root_run install -d -m 0755 "${registry_dir}"
	printf 'unqualified-search-registries = ["docker.io"]\n' |
		as_root_run tee "${registry_file}" >/dev/null
	as_root_run chmod 0644 "${registry_file}"
}

configure_fail2ban() {
	local family jail_file=/etc/fail2ban/jail.d/litepod-sshd.local
	if ! command -v fail2ban-client >/dev/null 2>&1; then
		family="$(distro_family)" || return 1
		pkg_refresh "${family}" as_root_run
		pkg_install "${family}" as_root_run fail2ban
	fi
	as_root_run install -d -m 0755 /etc/fail2ban/jail.d
	printf '[sshd]\nenabled = true\nbackend = systemd\nport = 22\nmaxretry = 5\nfindtime = 10m\nbantime = 1h\n' |
		as_root_run tee "${jail_file}" >/dev/null
	as_root_run systemctl enable --now fail2ban
	as_root_run systemctl restart fail2ban
}

litepod_image_repo="${LITEPOD_IMAGE_REPO:-docker.io/litepod/litepod}"
litepod_update_interval="${LITEPOD_UPDATE_INTERVAL:-3600}"
litepod_version_manifest_url="$(select_manifest_url "${env_file}")"
stable_tag_re='^v[0-9]+\.[0-9]+\.[0-9]+$'
alpha_tag_re='^v[0-9]+\.[0-9]+\.[0-9]+-alpha\.[0-9]+$'
core_tag_re='^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$'
manifest_resolved=false

validate_core_tag() {
	local name="$1" tag="$2"
	if [[ -z "${tag}" || "${tag}" == latest || ! "${tag}" =~ ${core_tag_re} ]]; then
		printf 'Version manifest returned an invalid %s tag: %s\n' "${name}" "${tag}" >&2
		return 1
	fi
}

resolve_litepod_image_tag() {
	local manifest_json resolved

	if ! manifest_json="$(curl -fsS --max-time 15 "${litepod_version_manifest_url}")"; then
		printf '%s\n' "Could not download litepod version manifest: ${litepod_version_manifest_url}" >&2
		printf 'Using fallback image tags: litepod=%s, caddy=%s, dragonfly=%s\n' \
			"${FALLBACK_LITEPOD_IMAGE_TAG}" \
			"${FALLBACK_CADDY_IMAGE_TAG}" \
			"${FALLBACK_DRAGONFLY_IMAGE_TAG}" >&2
		use_fallback_image_tags
		return 0
	fi
	if ! command -v python3 >/dev/null 2>&1; then
		printf '%s\n' 'python3 is required to read the litepod version manifest.' >&2
		return 1
	fi
	if ! resolved="$(printf '%s' "${manifest_json}" | REQUESTED_CHANNEL="${requested_channel}" python3 -c '
import json, os, sys
manifest = json.load(sys.stdin)
entry = manifest.get("channels", {}).get(os.environ["REQUESTED_CHANNEL"], {})
if not isinstance(entry, dict):
    entry = {}
def tag(name):
    value = entry.get(name, {})
    return value.get("tag", "") if isinstance(value, dict) else ""
print("\t".join((str(entry.get("version", "")), tag("caddy"), tag("dragonfly"))))
')"; then
		printf '%s\n' 'Version manifest is not valid JSON.' >&2
		return 1
	fi
	IFS=$'\t' read -r litepod_image_tag caddy_image_tag dragonfly_image_tag <<< "${resolved}"
	if [[ -z "${litepod_image_tag}" ]]; then
		printf '%s\n' "Version manifest does not contain a ${requested_channel} channel version." >&2
		return 1
	fi
	if [[ "${requested_channel}" == stable && ! "${litepod_image_tag}" =~ ${stable_tag_re} ]] || \
	   [[ "${requested_channel}" == alpha && ! "${litepod_image_tag}" =~ ${alpha_tag_re} ]]; then
		printf '%s\n' "Version manifest returned an invalid ${requested_channel} tag: ${litepod_image_tag}" >&2
		return 1
	fi
	validate_core_tag caddy "${caddy_image_tag}" || return 1
	validate_core_tag dragonfly "${dragonfly_image_tag}" || return 1
	return 0
}

if [[ -f "${env_file}" ]]; then
	current_channel="$(sed -n 's/^LITEPOD_UPDATE_CHANNEL=//p' "${env_file}" | head -n 1)"
	current_channel="${current_channel:-stable}"
	if [[ "${current_channel}" != "${requested_channel}" ]]; then
		printf 'Changing litepod update channel from %s to %s.\n' "${current_channel}" "${requested_channel}" >&2
		resolve_then_confirm_channel_wipe "${current_channel}" "${requested_channel}" resolve_litepod_image_tag wipe_channel_installation
	else
		resolve_litepod_image_tag
	fi
	manifest_resolved=true
fi

mkdir -p "${install_dir}"
chmod 755 "${install_dir}"
# cd out of the invoking shell's cwd (e.g. /root, unreadable to podman_user) so
# every later `runuser -u podman_user` below doesn't warn "cannot chdir to /root".
cd "${install_dir}"

if ! command -v curl >/dev/null 2>&1 || ! command -v openssl >/dev/null 2>&1 || \
   ! command -v podman >/dev/null 2>&1 || ! command -v podman-compose >/dev/null 2>&1; then
	install_packages
fi

configure_default_registry
configure_fail2ban

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

# ── Podman setup ─────────────────────────────────────────────────────────────

if [[ "${podman_mode}" == rootful ]]; then
	as_root_run systemctl enable --now podman.socket podman-restart.service
	as_root_run podman network exists litepod-network || as_root_run podman network create litepod-network
	podman_socket_path=/run/podman/podman.sock
else
	uid="$(id -u "${podman_user}")"
	if [[ "$(loginctl show-user "${podman_user}" -p Linger --value 2>/dev/null)" != yes ]]; then
		as_root_run loginctl enable-linger "${podman_user}"
	fi
	if ! systemctl is-active --quiet "user@${uid}.service" 2>/dev/null; then
		as_root_run systemctl start "user@${uid}.service" 2>/dev/null || true
	fi
	if [[ "${as_root}" == true ]]; then
		podman_user_run systemctl --user enable --now podman.socket
		podman_user_run systemctl --user enable --now podman-restart.service
	else
		export XDG_RUNTIME_DIR="/run/user/${uid}"
		export DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus"
		systemctl --user enable --now podman.socket
		systemctl --user enable --now podman-restart.service
	fi
	podman_user_run bash -c 'podman network exists litepod-network || podman network create litepod-network'
	podman_socket_path="/run/user/${uid}/podman/podman.sock"

	# Rootless Podman can't bind ports <1024 (80/443 for Caddy) unless the
	# kernel allows it. Lower the floor to 80 and persist it across reboots.
	unpriv_port_start="$(sysctl -n net.ipv4.ip_unprivileged_port_start 2>/dev/null || echo 1024)"
	if (( unpriv_port_start > 80 )); then
		printf 'Rootless Podman needs net.ipv4.ip_unprivileged_port_start <= 80 to bind :80/:443 (Caddy).\n'
		printf 'Currently %s — lowering it to 80 via sudo sysctl, persisted in /etc/sysctl.d.\n' "${unpriv_port_start}"
		as_root_run sysctl -w net.ipv4.ip_unprivileged_port_start=80
		printf 'net.ipv4.ip_unprivileged_port_start=80\n' | as_root_run tee /etc/sysctl.d/90-litepod-rootless-ports.conf >/dev/null

		unpriv_port_start="$(sysctl -n net.ipv4.ip_unprivileged_port_start 2>/dev/null || echo 1024)"
		if (( unpriv_port_start > 80 )); then
			printf '%s\n' "Warning: sysctl still reports ${unpriv_port_start} after the change — some hosts" >&2
			printf '%s\n' "(containers/restricted VMs) block this at the hypervisor level. Caddy will fail to" >&2
			printf '%s\n' "bind :80/:443 below. Ask your host provider to allow it, or run rootful Podman" >&2
			printf '%s\n' "instead (sudo bash install.sh -> choose rootful)." >&2
		else
			printf 'Confirmed: net.ipv4.ip_unprivileged_port_start is now %s.\n' "${unpriv_port_start}"
		fi
	fi
fi

if [[ ! -S "${podman_socket_path}" ]]; then
	printf '%s\n' "Podman's ${podman_mode} socket was not created at ${podman_socket_path}." >&2
	exit 1
fi

podman_socket="unix://${podman_socket_path}"

if [[ -t 1 ]]; then
	c_green=$'\e[1;32m'; c_red=$'\e[1;31m'; c_reset=$'\e[0m'
else
	c_green=''; c_red=''; c_reset=''
fi

if [[ "${podman_mode}" == rootless ]]; then
	printf 'Podman mode: %srootless%s (user: %s, socket: %s)\n' \
		"${c_green}" "${c_reset}" "${podman_user}" "${podman_socket}"
else
	printf 'Podman mode: %srootful%s (socket: %s)\n' \
		"${c_red}" "${c_reset}" "${podman_socket}"
fi

detect_public_ip() {
	local ip
	ip="$(curl -fsS4 --max-time 5 https://api.ipify.org 2>/dev/null)" \
		|| ip="$(curl -fsS4 --max-time 5 https://ifconfig.me 2>/dev/null)" \
		|| ip="$(curl -fsS4 --max-time 5 https://icanhazip.com 2>/dev/null | tr -d '[:space:]')" \
		|| true
	printf '%s' "${ip}"
}

if [[ "${manifest_resolved}" != true ]]; then
	resolve_litepod_image_tag
fi

resolve_resend_api_key() {
	local value="${RESEND_API_KEY:-}"
	if [[ -z "${value}" && -f "${env_file}" ]]; then
		value="$(sed -n 's/^RESEND_API_KEY=//p' "${env_file}" | head -n 1)"
	fi
	if [[ -n "${value}" ]]; then
		printf '%s' "${value}"
		return 0
	fi
	if { [[ -r /dev/tty ]] && [[ -w /dev/tty ]]; } 2>/dev/null; then
		printf 'Resend API key (required for email verification): ' > /dev/tty
		IFS= read -r -s value < /dev/tty
		printf '\n' > /dev/tty
	fi
	if [[ -z "${value}" ]]; then
		printf '%s\n' 'Resend API key is required for Litepod Cloud. Set RESEND_API_KEY and rerun the installer.' >&2
		return 1
	fi
	printf '%s' "${value}"
}

resend_api_key="$(resolve_resend_api_key)" || exit 1

resolve_turnstile_secret() {
	local value="${TURNSTILE_SECRET:-}"
	if [[ -z "${value}" && -f "${env_file}" ]]; then
		value="$(sed -n 's/^TURNSTILE_SECRET=//p' "${env_file}" | head -n 1)"
	fi
	if [[ -n "${value}" ]]; then
		printf '%s' "${value}"
		return 0
	fi
	if { [[ -r /dev/tty ]] && [[ -w /dev/tty ]]; } 2>/dev/null; then
		printf 'Cloudflare Turnstile secret key (required for signup bot protection): ' > /dev/tty
		IFS= read -r -s value < /dev/tty
		printf '\n' > /dev/tty
	fi
	if [[ -z "${value}" ]]; then
		printf '%s\n' 'Turnstile secret key is required for Litepod Cloud. Set TURNSTILE_SECRET and rerun the installer.' >&2
		return 1
	fi
	printf '%s' "${value}"
}

turnstile_secret="$(resolve_turnstile_secret)" || exit 1

if [[ ! -f "${env_file}" ]]; then
 hash_secret="$(openssl rand -hex 32)"
	encryption_key="$(openssl rand -hex 32)"
	metrics_username="admin"
	metrics_password="$(openssl rand -hex 16)"
	dfly_password="$(openssl rand -hex 32)"
	public_ip="${LITEPOD_PUBLIC_IP:-$(detect_public_ip)}"

	if [[ -z "${public_ip}" ]]; then
		printf '%s\n' "Could not auto-detect the host's public IP (no outbound internet?)." >&2
		printf '%s\n' "Set LITEPOD_PUBLIC_IP and rerun this script." >&2
		exit 1
	fi

	# Bootstrap-only value: GitHub App callbacks/webhooks use this until you set a real
	# domain in Settings -> Server Domain after first login, which takes over from then on.
	# No explicit :80 here - browsers omit the default port from the Origin header they
	# send, and CORS_ORIGINS/origin_guard compare against this value verbatim.
	public_url="${PUBLIC_URL:-http://${public_ip}}"

	cat > "${env_file}" <<-EOF
 HASH_SECRET=${hash_secret}
	ENCRYPTION_KEY=${encryption_key}
	METRICS_USERNAME=${metrics_username}
	METRICS_PASSWORD=${metrics_password}
	DFLY_PASSWORD=${dfly_password}
	RESEND_API_KEY=${resend_api_key}
	TURNSTILE_SECRET=${turnstile_secret}
	LITEPOD_PUBLIC_IP=${public_ip}
	PUBLIC_URL=${public_url}
	LITEPOD_IMAGE_REPO=${litepod_image_repo}
	LITEPOD_IMAGE_TAG=${litepod_image_tag}
	LITEPOD_UPDATE_CHANNEL=${requested_channel}
	LITEPOD_VERSION_MANIFEST_URL=${litepod_version_manifest_url}
	LITEPOD_UPDATE_INTERVAL=${litepod_update_interval}
 CADDY_IMAGE_TAG=${caddy_image_tag}
	DRAGONFLY_IMAGE_TAG=${dragonfly_image_tag}
	IS_CLOUD=true
	# Polar.sh billing is optional until credentials are added in this Cloud-only .env.
	POLAR_ENVIRONMENT=${POLAR_ENVIRONMENT:-production}
	POLAR_ACCESS_TOKEN=${POLAR_ACCESS_TOKEN:-}
	POLAR_WEBHOOK_SECRET=${POLAR_WEBHOOK_SECRET:-}
	# Remote API logging is Cloud-only by default. Operators can leave LOKI_URL
	# blank to retain stdout JSON logs without shipping them.
	LOKI_URL=${LOKI_URL:-https://loki.sebasgc.xyz}
	LOKI_SERVICE=${LOKI_SERVICE:-litepod-cloud}
	EOF
	chmod 600 "${env_file}"

	printf '%s\n' "Created ${env_file}. Auto-generated JWT key/token, HASH_SECRET, ENCRYPTION_KEY," >&2
	printf '%s\n' "and detected public IP ${public_ip}. Continuing install..." >&2
fi

upsert_env_value() {
	local key="$1" value="$2"
	if grep -q "^${key}=" "${env_file}"; then
		sed -i "s|^${key}=.*|${key}=${value}|" "${env_file}"
	else
		printf '%s=%s\n' "${key}" "${value}" >> "${env_file}"
	fi
}

upsert_env_value LITEPOD_IMAGE_REPO "${litepod_image_repo}"
upsert_env_value LITEPOD_IMAGE_TAG "${litepod_image_tag}"
upsert_env_value LITEPOD_UPDATE_CHANNEL "${requested_channel}"
upsert_env_value LITEPOD_VERSION_MANIFEST_URL "${litepod_version_manifest_url}"
upsert_env_value CADDY_IMAGE_TAG "${caddy_image_tag}"
upsert_env_value DRAGONFLY_IMAGE_TAG "${dragonfly_image_tag}"
upsert_env_value IS_CLOUD true
upsert_env_value RESEND_API_KEY "${resend_api_key}"
upsert_env_value TURNSTILE_SECRET "${turnstile_secret}"

# Polar is intentionally Cloud-only. Preserve existing secrets on upgrades;
# an operator may provide any value through the installer environment.
ensure_cloud_env_value() {
	local key="$1" default_value="$2" provided="${!1:-}"
	if [[ -n "${provided}" ]]; then
		upsert_env_value "${key}" "${provided}"
	elif ! grep -q "^${key}=" "${env_file}"; then
		upsert_env_value "${key}" "${default_value}"
	fi
}
ensure_cloud_env_value POLAR_ENVIRONMENT production
ensure_cloud_env_value POLAR_ACCESS_TOKEN ''
ensure_cloud_env_value POLAR_WEBHOOK_SECRET ''
ensure_cloud_env_value LOKI_URL https://loki.sebasgc.xyz
ensure_cloud_env_value LOKI_SERVICE litepod-cloud
if ! grep -q '^DFLY_PASSWORD=.' "${env_file}"; then
	upsert_env_value DFLY_PASSWORD "$(openssl rand -hex 32)"
fi

sed -i '/^LITEPOD_PODMAN_SOCKET=/d; /^LITEPOD_PODMAN_SOCKET_PATH=/d' "${env_file}"
printf 'LITEPOD_PODMAN_SOCKET=%s\nLITEPOD_PODMAN_SOCKET_PATH=%s\n' \
	"${podman_socket}" "${podman_socket_path}" >> "${env_file}"

mkdir -p "${install_dir}/railpack-builder/scripts"

cat > "${install_dir}/railpack-builder/Dockerfile" <<-'EOF'
# syntax=docker/dockerfile:1
# Ephemeral build container used by Railpack deploys. The API spawns one of
# these per deploy over the Podman socket, so no build tooling is ever needed on
# the host or in the (distroless) API runtime image.

FROM docker.io/moby/buildkit:rootless AS buildkit

# Debian rather than Alpine: `railpack prepare` downloads a glibc-linked mise
# binary at runtime, which cannot execute on musl.
FROM docker.io/library/debian:bookworm-slim
ARG RAILPACK_VERSION=v0.31.1

COPY --from=buildkit /usr/bin/buildctl /usr/local/bin/buildctl
COPY --from=buildkit /usr/bin/buildkitd /usr/local/bin/buildkitd
COPY --from=buildkit /usr/bin/buildkit-runc /usr/local/bin/buildkit-runc

RUN apt-get update \
 && apt-get install -y --no-install-recommends curl ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && install -d /workspace/source /workspace/inputs /output \
 && case "$(uname -m)" in \
      x86_64)  RAILPACK_ARCH=x86_64 ;; \
      aarch64|arm64) RAILPACK_ARCH=arm64 ;; \
      *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;; \
    esac \
 && curl -fsSL "https://github.com/railwayapp/railpack/releases/download/${RAILPACK_VERSION}/railpack-${RAILPACK_VERSION}-${RAILPACK_ARCH}-unknown-linux-musl.tar.gz" \
    | tar -xz -C /usr/local/bin railpack \
 && chmod +x /usr/local/bin/railpack \
 && railpack --version

COPY scripts/railpack-prepare.sh /usr/local/bin/railpack-prepare
COPY scripts/railpack-build.sh /usr/local/bin/railpack-build
RUN chmod +x /usr/local/bin/railpack-prepare /usr/local/bin/railpack-build

ENTRYPOINT ["/usr/local/bin/railpack-prepare"]
EOF

cat > "${install_dir}/railpack-builder/scripts/railpack-prepare.sh" <<-'EOF'
#!/bin/sh
# Generates a Railpack plan with access to application data. The actual build
# runs later in an unprivileged container with no /data mount.
set -eu

: "${CONTEXT_DIR:?CONTEXT_DIR is required}"
: "${OUT_DIR:?OUT_DIR is required}"

ENV_FILE="$OUT_DIR/build-env"
KEYS=""
if [ -f "$ENV_FILE" ]; then
    KEYS=$(cut -d= -f1 < "$ENV_FILE" | grep -v '^$' || true)
fi

echo "==> Preparing build plan"
set --
if [ -n "$KEYS" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
        [ -z "$line" ] && continue
        export "$line"
        set -- "$@" --env "$line"
    done < "$ENV_FILE"
fi
railpack prepare "$CONTEXT_DIR" --plan-out "$OUT_DIR/railpack-plan.json" "$@"
EOF

cat > "${install_dir}/railpack-builder/scripts/railpack-build.sh" <<-'EOF'
#!/bin/sh
# Runs in the isolated, non-root Railpack build container. The API transfers
# only the prepared build inputs into this container; it never mounts /data.
set -eu

: "${IMAGE_TAG:?IMAGE_TAG is required}"
: "${CONTEXT_DIR:?CONTEXT_DIR is required}"
: "${PLAN_DIR:?PLAN_DIR is required}"
: "${OUT_DIR:?OUT_DIR is required}"

RAILPACK_FRONTEND="${RAILPACK_FRONTEND:-ghcr.io/railwayapp/railpack-frontend}"
BUILDKIT_ADDR="unix:///tmp/buildkit-runtime/buildkitd.sock"
BUILDKIT_LOG="$OUT_DIR/buildkitd.log"

# A rootless daemon per build keeps BuildKit's workers in this temporary
# container's cgroup. Podman has already placed this container in a rootless
# user namespace, so BuildKit runs as that mapped root rather than host root.
# The API applies the application's CPU/RAM limits to that cgroup. The native
# snapshotter avoids granting /dev/fuse to the build.
export XDG_RUNTIME_DIR=/tmp/buildkit-runtime
mkdir -p "$XDG_RUNTIME_DIR"
buildkitd --rootless --addr "$BUILDKIT_ADDR" \
    --root /tmp/buildkit \
    --oci-worker-snapshotter=native >"$BUILDKIT_LOG" 2>&1 &
BUILDKIT_PID=$!
cleanup_buildkit() {
    kill "$BUILDKIT_PID" 2>/dev/null || true
    wait "$BUILDKIT_PID" 2>/dev/null || true
}
trap cleanup_buildkit EXIT INT TERM

while [ ! -S /tmp/buildkit-runtime/buildkitd.sock ]; do
    if ! kill -0 "$BUILDKIT_PID" 2>/dev/null; then
        cat "$BUILDKIT_LOG" >&2 || true
        exit 1
    fi
    sleep 0.1
done

# Build-time environment. The API writes one KEY=VALUE per line so values never
# appear in argv (visible to anything that can read the process table).
ENV_FILE="$PLAN_DIR/build-env"
KEYS=""
if [ -f "$ENV_FILE" ]; then
    KEYS=$(cut -d= -f1 < "$ENV_FILE" | grep -v '^$' || true)
fi

if [ -n "$KEYS" ]; then
    while IFS= read -r line || [ -n "$line" ]; do
        [ -z "$line" ] && continue
        export "$line"
    done < "$ENV_FILE"
fi

# Railpack compiles those variables into BuildKit secret references, so every
# one of them has to be handed to buildctl as a secret or the build aborts with
# "secret NAME: not found". Only the names go through argv; buildctl reads the
# values from the environment exported above.
set --
for key in $KEYS; do
    set -- "$@" --secret "id=$key,env=$key"
done

echo "==> Building image $IMAGE_TAG"
buildctl --addr "$BUILDKIT_ADDR" build \
    --progress plain \
    --no-cache \
    --local "context=$CONTEXT_DIR" \
    --local "dockerfile=$PLAN_DIR" \
    --frontend=gateway.v0 \
    --opt "source=$RAILPACK_FRONTEND" \
    --opt filename=railpack-plan.json \
    "$@" \
    --output "type=docker,name=$IMAGE_TAG,dest=$OUT_DIR/image.tar"

echo "==> Build finished"
EOF
chmod +x "${install_dir}/railpack-builder/scripts/railpack-build.sh"
chmod +x "${install_dir}/railpack-builder/scripts/railpack-prepare.sh"

if ! podman_user_run podman image exists localhost/litepod-railpack-builder:rootless-v2; then
	printf '%s\n' "Building Railpack builder image..."
	podman_user_run podman build -f "${install_dir}/railpack-builder/Dockerfile" -t localhost/litepod-railpack-builder:rootless-v2 "${install_dir}/railpack-builder"
fi

use_caddy_sockets=false
caddy_socket_activation_supported && use_caddy_sockets=true
caddy_bind=''
if [[ "${use_caddy_sockets}" == true ]]; then
	caddy_bind=$'\tbind fd/3\n'
fi

cat > "${install_dir}/Caddyfile" <<-EOF
{
	admin 0.0.0.0:2019
}

:80 {
${caddy_bind}	# Container name, not the \`app\` service name - a deployed app's Compose file on
	# this network could claim \`app\` as a DNS alias and hijack the dashboard.
	reverse_proxy litepod-api:6001
}
EOF

cat > "${compose_file}" <<-EOF
services:
  caddy:
    image: docker.io/library/caddy:\${CADDY_IMAGE_TAG:-${FALLBACK_CADDY_IMAGE_TAG}}
    container_name: litepod-caddy
    ports:
      - "80:80"
      - "443:443"
    volumes:
      - ./Caddyfile:/etc/caddy/Caddyfile:ro
      - caddy_data:/data
      - caddy_config:/config
    restart: unless-stopped

  dragonfly:
    image: docker.dragonflydb.io/dragonflydb/dragonfly:\${DRAGONFLY_IMAGE_TAG:-${FALLBACK_DRAGONFLY_IMAGE_TAG}}
    container_name: litepod-dragonfly
    restart: unless-stopped
    environment:
      DFLY_requirepass: \${DFLY_PASSWORD:?Set DFLY_PASSWORD in .env}

  app:
    image: \${LITEPOD_IMAGE_REPO:-docker.io/litepod/litepod}:\${LITEPOD_IMAGE_TAG:-${FALLBACK_LITEPOD_IMAGE_TAG}}
    container_name: litepod-api
    environment:
      APP_ENV: production
      IS_CLOUD: "true"
      PORT: "6001"
      UI_DIST_PATH: /app/ui
      DATABASE_PATH: /var/lib/litepod/litepod.db
      HASH_SECRET: \${HASH_SECRET:?Set HASH_SECRET in .env}
      ENCRYPTION_KEY: \${ENCRYPTION_KEY:?Set ENCRYPTION_KEY in .env}
      METRICS_USERNAME: \${METRICS_USERNAME:?Set METRICS_USERNAME in .env}
      METRICS_PASSWORD: \${METRICS_PASSWORD:?Set METRICS_PASSWORD in .env}
      CADDY_URL: http://litepod-caddy:2019
      CADDY_ADMIN_URL: http://litepod-caddy:2019
      DRAGONFLY_URL: redis://:\${DFLY_PASSWORD:?Set DFLY_PASSWORD in .env}@litepod-dragonfly:6379
      PODMAN_SOCKET: \${LITEPOD_PODMAN_SOCKET:?Set by scripts/install.sh}
      LITEPOD_DATA_DIR: /var/lib/litepod
      LITEPOD_DATA_VOLUME: litepod_data
      LITEPOD_PODMAN_NETWORK: litepod-network
      # The self-updater recreates this container under this name. Without it the
      # name is probed at boot, and a wrong answer costs Caddy's \`litepod-api:6001\`
      # catch-all.
      LITEPOD_CONTAINER_NAME: litepod-api
      LITEPOD_PUBLIC_IP: \${LITEPOD_PUBLIC_IP:?Set LITEPOD_PUBLIC_IP in .env}
      PUBLIC_URL: \${PUBLIC_URL:?Set PUBLIC_URL in .env}
      CORS_ORIGINS: \${PUBLIC_URL:?Set PUBLIC_URL in .env}
      RESEND_API_KEY: \${RESEND_API_KEY:?Set RESEND_API_KEY in .env}
      TURNSTILE_SECRET: \${TURNSTILE_SECRET:?Set TURNSTILE_SECRET in .env}
      # Optional Cloud Billing credentials. They are absent from self-hosted compose.
      POLAR_ENVIRONMENT: \${POLAR_ENVIRONMENT:-production}
      POLAR_ACCESS_TOKEN: \${POLAR_ACCESS_TOKEN:-}
      POLAR_WEBHOOK_SECRET: \${POLAR_WEBHOOK_SECRET:-}
      LOKI_URL: \${LOKI_URL:-}
      LOKI_SERVICE: \${LOKI_SERVICE:-litepod-cloud}
      LITEPOD_IMAGE_REPO: \${LITEPOD_IMAGE_REPO:-docker.io/litepod/litepod}
      LITEPOD_IMAGE_TAG: \${LITEPOD_IMAGE_TAG:-${FALLBACK_LITEPOD_IMAGE_TAG}}
      LITEPOD_UPDATE_CHANNEL: \${LITEPOD_UPDATE_CHANNEL:-stable}
      LITEPOD_VERSION_MANIFEST_URL: \${LITEPOD_VERSION_MANIFEST_URL:-https://litepod.sh/version.json}
      LITEPOD_UPDATE_INTERVAL: \${LITEPOD_UPDATE_INTERVAL:-3600}
      LITEPOD_STACK_ENV_FILE: /etc/litepod-stack/.env
    volumes:
      - \${LITEPOD_PODMAN_SOCKET_PATH:?Set by scripts/install.sh}:\${LITEPOD_PODMAN_SOCKET_PATH:?Set by scripts/install.sh}
      - litepod_data:/var/lib/litepod
      - ./:/etc/litepod-stack:rw
    depends_on:
      - caddy
      - dragonfly
    restart: unless-stopped

volumes:
  caddy_data:
  caddy_config:
  litepod_data:
    name: litepod_data

networks:
  default:
    external: true
    name: litepod-network
EOF

# Files above were written by root; the self-updater edits .env as the Podman user.
if [[ "${as_root}" == true && "${podman_mode}" == rootless ]]; then
	chown -R "${podman_user}:" "${install_dir}"
fi

if [[ "${use_caddy_sockets}" == true ]]; then
	# Caddy is a Quadlet unit fed by litepod-caddy.socket, not a Compose service.
	sed -i '/^  caddy:$/,/^$/d; /^      - caddy$/d' "${compose_file}"
fi

podman_user_run podman pull "${litepod_image_repo}:${litepod_image_tag}"
# The self-updater recreates core containers without Compose labels, so `up`
# hits a name clash and restarts the old container with stale env. Remove them
# by name (named volumes survive) so every rerun applies this compose file.
# From a root shell, rootless `rm` can remove the container yet fail tearing
# down its netns ("kill network process: permission denied"); trust absence.
for name in litepod-api litepod-caddy litepod-dragonfly; do
	podman_user_run podman rm -f --ignore "${name}" || podman_object_is_absent container "${name}" || {
		printf '%s\n' "Could not remove ${name}; refusing to continue." >&2
		exit 1
	}
done
podman_user_run podman-compose -f "${compose_file}" up -d --remove-orphans
podman_user_run podman-compose -f "${compose_file}" ps
if [[ "${use_caddy_sockets}" == true ]]; then
	configure_caddy_socket_activation
fi
if [[ "${podman_mode}" == rootless ]]; then
	configure_rootless_stack_service
fi

reported_ip="$(grep -m1 '^LITEPOD_PUBLIC_IP=' "${env_file}" | cut -d= -f2-)"

printf '\n%s\n' "litepod is running."
printf '%s\n' "Local check:  curl http://localhost/health"
printf '%s\n' "From outside: http://${reported_ip}/  (and http://${reported_ip}/health)"
printf '%s\n' "Install dir: ${install_dir}  (env file: ${env_file})"
printf '%s\n' "Channel: ${requested_channel}  (tag: ${litepod_image_tag})"
printf '%s\n' "Log in and set the real domain under Settings -> Server Domain — the auto-detected"
printf '%s\n' "IP above is only a bootstrap fallback for GitHub App callbacks/webhooks until then."
printf '\n%s\n' "Check containers/ports anytime with:"
if [[ "${as_root}" == true && "${podman_mode}" == rootless ]]; then
	podman_user_uid="$(id -u "${podman_user}")"
	printf '%s\n' "  su - ${podman_user} -c 'podman ps'"
	printf '%s\n' "  sudo -u ${podman_user} env XDG_RUNTIME_DIR=/run/user/${podman_user_uid} DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/${podman_user_uid}/bus systemctl --user status litepod-stack.service"
else
	printf '%s\n' "  podman ps"
	if [[ "${podman_mode}" == rootless ]]; then
		printf '%s\n' "  systemctl --user status litepod-stack.service"
	fi
fi
printf '%s\n' "Expect 3 containers Up: litepod-caddy (80->80, 443->443), litepod-dragonfly (internal only, 6379),"
printf '%s\n' "and litepod-api (internal only, 6001, reached through Caddy)."
}

install_script_source="${BASH_SOURCE[0]-}"
if should_run_installer_main "${install_script_source}" "$0"; then
	main "$@"
fi
