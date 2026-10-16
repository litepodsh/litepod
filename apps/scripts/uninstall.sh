#!/usr/bin/env bash
# litepod production uninstaller — reverses scripts/install.sh safely.
#
# A default uninstall removes litepod, its deployed applications, and all their
# managed data. --keep-applications preserves deployed applications and data.
set -euo pipefail

usage() {
	printf '%s\n' 'Usage: bash uninstall.sh [--keep-data] [--keep-applications|--purge-applications]'
}

parse_uninstall_args() {
	keep_data=false
	application_policy=purge
	application_policy_set=false
	while [[ "$#" -gt 0 ]]; do
		case "$1" in
		--keep-data) keep_data=true ;;
		--keep-applications)
			[[ "${application_policy_set}" == false ]] || { printf '%s\n' 'Choose only one application policy.' >&2; return 1; }
			application_policy=keep
			application_policy_set=true
			;;
		--purge-applications)
			[[ "${application_policy_set}" == false ]] || { printf '%s\n' 'Choose only one application policy.' >&2; return 1; }
			application_policy=purge
			application_policy_set=true
			;;
		-h|--help) usage; return 2 ;;
		*) usage >&2; return 1 ;;
		esac
		shift
	done
	if [[ "${keep_data}" == true && "${application_policy_set}" == true ]]; then
		printf '%s\n' '--keep-data cannot be combined with an application policy.' >&2
		return 1
	fi
}

podman_object_is_absent() {
	local kind="$1" name="$2" status
	if podman_user_run podman "${kind}" exists "${name}"; then
		return 1
	else
		status=$?
	fi
	[[ "${status}" -eq 1 ]]
}

core_containers_are_absent() {
	local name
    for name in litepod-caddy litepod-dragonfly litepod-api; do
		podman_object_is_absent container "${name}" || return 1
	done
}

core_volumes_are_absent() {
	local name
    for name in litepod_caddy_data litepod_caddy_config litepod_data; do
		podman_object_is_absent volume "${name}" || return 1
	done
}

cleanup_core_stack() {
	if ! podman_user_run podman-compose -f "${compose_file}" down --remove-orphans; then
		printf '%s\n' "First 'down' pass hit a rootless-netns cleanup race (known Podman issue) — retrying..." >&2
		sleep 2
		podman_user_run podman-compose -f "${compose_file}" down --remove-orphans || true
	fi
	# Podman can report before its rootless network helper releases container
	# state. Give it a short grace period, then require positive absence.
	sleep 2
	if core_containers_are_absent; then
		return 0
	fi
	printf '%s\n' 'Compose cleanup failed and litepod core containers may remain; retaining the installation directory and .env.' >&2
	return 1
}

remove_core_volumes() {
	local volume
    for volume in litepod_caddy_data litepod_caddy_config; do
		if podman_user_run podman volume rm -f "${volume}"; then
			continue
		fi
		if ! podman_object_is_absent volume "${volume}"; then
			printf '%s\n' "Could not remove or verify absence of ${volume}; retaining the installation directory and .env." >&2
			return 1
		fi
	done
}

remove_litepod_data_volume() {
	if podman_user_run podman volume rm -f litepod_data; then
		return 0
	fi
	if podman_object_is_absent volume litepod_data; then
		return 0
	fi
	printf '%s\n' 'Could not remove or verify absence of litepod_data; retaining the installation directory and .env.' >&2
	return 1
}

stop_rootless_stack_service() {
	local user_home unit_file
	[[ "${podman_mode}" == rootless ]] || return 0
	user_home="$(getent passwd "${podman_user}" | cut -d: -f6)"
	[[ -n "${user_home}" ]] || { printf '%s\n' "Could not determine home directory for ${podman_user}." >&2; return 1; }
	unit_file="${user_home}/.config/systemd/user/litepod-stack.service"
	if ! podman_user_run systemctl --user disable --now litepod-stack.service; then
		if [[ -f "${unit_file}" ]] || podman_user_run systemctl --user is-active --quiet litepod-stack.service; then
			printf '%s\n' 'Could not stop litepod-stack.service; retaining the installation directory and .env.' >&2
			return 1
		fi
	fi
	if [[ -f "${unit_file}" ]]; then
		podman_user_run rm -f -- "${unit_file}" || return 1
	fi
	podman_user_run systemctl --user daemon-reload
}

discover_deployed_workloads() {
	local seeds members name member label project
	workload_containers=()
	workload_projects=()
	if ! seeds="$(podman_user_run podman ps -a --filter network=litepod-network --format '{{.Names}}')"; then
		printf '%s\n' 'Could not list application containers on litepod-network.' >&2
		return 1
	fi
	while IFS= read -r name; do
		[[ -n "${name}" ]] && add_unique_workload_container "${name}"
	done <<< "${seeds}"
	for name in "${workload_containers[@]}"; do
		for label in com.docker.compose.project io.podman.compose.project; do
			if ! project="$(podman_user_run podman inspect --format "{{with .Config.Labels}}{{index . \\\"${label}\\\"}}{{end}}" "${name}")"; then
				printf '%s\n' "Could not inspect workload container ${name}." >&2
				return 1
			fi
			[[ -z "${project}" || "${project}" == '<no value>' ]] && continue
			add_unique_workload_project "${project}"
			if ! members="$(podman_user_run podman ps -a --filter "label=${label}=${project}" --format '{{.Names}}')"; then
				printf '%s\n' "Could not list Compose project ${project}." >&2
				return 1
			fi
			while IFS= read -r member; do
				[[ -n "${member}" ]] && add_unique_workload_container "${member}"
			done <<< "${members}"
		done
	done
}

add_unique_workload_container() {
	local candidate="$1" name
	for name in "${workload_containers[@]}"; do [[ "${name}" == "${candidate}" ]] && return; done
	workload_containers+=("${candidate}")
}

add_unique_workload_project() {
	local candidate="$1" project
	for project in "${workload_projects[@]}"; do [[ "${project}" == "${candidate}" ]] && return; done
	workload_projects+=("${candidate}")
}

is_discovered_workload_project() {
	local candidate="$1" project
	for project in "${workload_projects[@]}"; do [[ "${project}" == "${candidate}" ]] && return 0; done
	return 1
}

choose_application_policy() {
	# The default is intentionally destructive. The only opt-out is the explicit
	# --keep-applications flag parsed above.
	return 0
}

purge_deployed_workloads() {
	local mounts all_volumes name volume project label
	local -a volumes=()
	for name in "${workload_containers[@]}"; do
		podman_object_is_absent container "${name}" && continue
		if ! mounts="$(podman_user_run podman inspect --format '{{range .Mounts}}{{if eq .Type "volume"}}{{println .Name}}{{end}}{{end}}' "${name}")"; then
			printf '%s\n' "Could not inspect volumes for workload container ${name}." >&2
			return 1
		fi
		while IFS= read -r volume; do [[ -n "${volume}" ]] && volumes+=("${volume}"); done <<< "${mounts}"
	done
	if ! all_volumes="$(podman_user_run podman volume ls --format '{{.Name}}')"; then
		printf '%s\n' 'Could not list application volumes.' >&2
		return 1
	fi
	while IFS= read -r volume; do
		[[ -z "${volume}" ]] && continue
		if [[ "${volume}" == litepod-managed-* ]]; then volumes+=("${volume}"); continue; fi
		project=''
		for label in com.docker.compose.project io.podman.compose.project; do
			if ! project="$(podman_user_run podman volume inspect --format "{{with .Labels}}{{index . \\\"${label}\\\"}}{{end}}" "${volume}")"; then
				printf '%s\n' "Could not inspect volume ${volume}." >&2
				return 1
			fi
			[[ -n "${project}" && "${project}" != '<no value>' ]] && break
		done
		is_discovered_workload_project "${project}" && volumes+=("${volume}")
	done <<< "${all_volumes}"
	for name in "${workload_containers[@]}"; do
		if ! podman_user_run podman rm -f "${name}" && ! podman_object_is_absent container "${name}"; then
			printf '%s\n' "Could not remove workload container ${name}." >&2
			return 1
		fi
	done
	# Do not race Podman's asynchronous container cleanup with volume removal.
	sleep 3
	for name in "${workload_containers[@]}"; do
		if ! podman_object_is_absent container "${name}"; then
			printf '%s\n' "Workload container ${name} still exists; refusing to remove its volumes." >&2
			return 1
		fi
	done
	for volume in "${volumes[@]}"; do
		if ! podman_user_run podman volume rm -f "${volume}" && ! podman_object_is_absent volume "${volume}"; then
			printf '%s\n' "Could not remove workload volume ${volume}." >&2
			return 1
		fi
	done
}

remove_empty_litepod_network() {
	local remaining
	if podman_user_run podman network exists litepod-network; then
		remaining="$(podman_user_run podman ps -a --filter network=litepod-network --format '{{.Names}}')" || return 1
		if [[ -n "${remaining}" ]]; then
			printf '%s\n' 'litepod-network still has attached containers; keeping it.' >&2
			return 0
		fi
		podman_user_run podman network rm litepod-network
	fi
}

find_install_dir() {
	local c hint_user hint_home f
	candidates=()
	[[ -n "${LITEPOD_INSTALL_DIR:-}" ]] && candidates+=("${LITEPOD_INSTALL_DIR}")
	candidates+=(/opt/litepod)
	if [[ "${as_root}" == true ]]; then
		hint_user="${LITEPOD_PODMAN_USER:-${SUDO_USER:-}}"
		[[ "${hint_user}" == root ]] && hint_user=''
		if [[ -n "${hint_user}" ]]; then
			hint_home="$(getent passwd "${hint_user}" | cut -d: -f6)"
			[[ -n "${hint_home}" ]] && candidates+=("${hint_home}/litepod")
		fi
	else
		candidates+=("${HOME}/litepod")
	fi
	install_dir=''
	for c in "${candidates[@]}"; do
		if [[ -f "${c}/podman-compose-prod.yml" ]]; then install_dir="${c}"; break; fi
	done
	if [[ -z "${install_dir}" ]]; then
		local -a found=()
		while IFS= read -r -d '' f; do found+=("${f}"); done < <(find /home /root -maxdepth 3 -name podman-compose-prod.yml -path '*/litepod/podman-compose-prod.yml' -print0 2>/dev/null)
		if [[ ${#found[@]} -eq 1 ]]; then
			install_dir="$(dirname "${found[0]}")"
		elif [[ ${#found[@]} -gt 1 ]]; then
			printf '%s\n' 'Found multiple litepod installs; set LITEPOD_INSTALL_DIR to choose one.' >&2
			return 1
		fi
	fi
	if [[ -z "${install_dir}" ]]; then
		printf '%s\n' "Could not find a litepod install (checked ${candidates[*]})." >&2
		return 1
	fi
	compose_file="${install_dir}/podman-compose-prod.yml"
	env_file="${install_dir}/.env"
}

configure_podman_access() {
	local socket_path_line
	socket_path_line="$(grep -m1 '^LITEPOD_PODMAN_SOCKET_PATH=' "${env_file}" 2>/dev/null | cut -d= -f2-)"
	if [[ "${socket_path_line}" == /run/podman/podman.sock ]]; then
		podman_mode=rootful; podman_user=''
	elif [[ "${socket_path_line}" =~ ^/run/user/([0-9]+)/podman/podman\.sock$ ]]; then
		podman_mode=rootless; podman_user="$(getent passwd "${BASH_REMATCH[1]}" | cut -d: -f1)"
	else
		printf '%s\n' "Could not read Podman mode from ${env_file}; falling back to directory ownership." >&2
		podman_mode=rootless; podman_user="$(stat -c '%U' "${install_dir}")"
	fi
	if [[ "${as_root}" == false && "${podman_user}" != "$(id -un)" ]]; then
		printf '%s\n' "This install runs Podman as ${podman_user} — rerun as root, or as ${podman_user}." >&2
		return 1
	fi
}

as_root_run() {
	if [[ "${as_root}" == true ]]; then "$@"; else sudo "$@"; fi
}

podman_user_run() {
	if [[ "${as_root}" == true && "${podman_mode}" == rootless ]]; then
		local uid
		uid="$(id -u "${podman_user}")"
		runuser -u "${podman_user}" -- env XDG_RUNTIME_DIR="/run/user/${uid}" DBUS_SESSION_BUS_ADDRESS="unix:path=/run/user/${uid}/bus" "$@"
	else
		"$@"
	fi
}

cleanup_keep_data() {
	podman_user_run podman-compose -f "${compose_file}" down --remove-orphans || return 1
	printf '%s\n' "Containers were removed; litepod data, ${install_dir}, applications, and litepod-network were kept."
}

remove_install_dir() {
	if [[ "${install_dir}" != /opt/litepod && "${install_dir}" != */litepod ]]; then
		printf '%s\n' "Refusing to delete unexpected installation directory: ${install_dir}" >&2
		return 1
	fi
	cd /tmp
	rm -rf -- "${install_dir}"
}

remove_local_console_access() {
	local console_user console_home authorized_file
	console_user="$(sed -n 's/^LITEPOD_LOCAL_SSH_USER=//p' "${env_file}" | head -n1)"
	[[ -n "${console_user}" && "${console_user}" != root ]] || return 0
	console_home="$(getent passwd "${console_user}" | cut -d: -f6)"
	[[ -n "${console_home}" ]] || return 0
	authorized_file="${console_home}/.ssh/authorized_keys"
	if [[ -f "${authorized_file}" ]]; then
		as_root_run sed -i '/ litepod-local-console$/d' "${authorized_file}"
	fi
	if [[ -d "${install_dir}/host-console" ]]; then
		rm -f -- "${install_dir}/host-console/id_ed25519" "${install_dir}/host-console/id_ed25519.pub" "${install_dir}/host-console/known_hosts"
		rmdir "${install_dir}/host-console" 2>/dev/null || true
	fi
}

revert_rootless_host_tweaks() {
	[[ "${podman_mode}" == rootless ]] || return 0
	if [[ -f /etc/sysctl.d/90-litepod-rootless-ports.conf ]]; then
		as_root_run rm -f /etc/sysctl.d/90-litepod-rootless-ports.conf
		as_root_run sysctl -w net.ipv4.ip_unprivileged_port_start=1024 >/dev/null
	fi
	if [[ "$(loginctl show-user "${podman_user}" -p Linger --value 2>/dev/null)" == yes ]]; then
		as_root_run loginctl disable-linger "${podman_user}"
	fi
}

main() {
	local parse_status
	if parse_uninstall_args "$@"; then
		:
	else
		parse_status=$?
		[[ "${parse_status}" -eq 2 ]] && return 0
		return 1
	fi
	[[ "${EUID}" -eq 0 ]] && as_root=true || as_root=false
	find_install_dir || return 1
	configure_podman_access || return 1
	printf '%s\n' "Found install at ${install_dir} (Podman mode: ${podman_mode}${podman_user:+, user: ${podman_user}})"
	if [[ "${keep_data}" == true ]]; then cleanup_keep_data; return; fi
	discover_deployed_workloads || return 1
	choose_application_policy || return 1
	stop_rootless_stack_service || return 1
	cleanup_core_stack || return 1
	if [[ "${application_policy}" == purge ]]; then
		purge_deployed_workloads || return 1
		remove_litepod_data_volume || return 1
	fi
	remove_core_volumes || return 1
	core_containers_are_absent || { printf '%s\n' 'Core litepod containers remain; retaining the installation directory and .env.' >&2; return 1; }
	if [[ "${application_policy}" == purge ]]; then
		core_volumes_are_absent || { printf '%s\n' 'litepod volumes remain; retaining the installation directory and .env.' >&2; return 1; }
	fi
	podman_user_run podman rmi localhost/litepod-railpack-builder:rootless-v2 2>/dev/null || true
	if [[ "${application_policy}" == purge ]]; then remove_empty_litepod_network || return 1; fi
	remove_local_console_access || return 1
	remove_install_dir || return 1
	if [[ "${application_policy}" == purge || ${#workload_containers[@]} -eq 0 ]]; then revert_rootless_host_tweaks; fi
	if [[ "${application_policy}" == purge ]]; then
		printf '%s\n' 'litepod, deployed applications, and their managed data were removed.'
	else
		printf '%s\n' 'litepod core services and credentials were removed; deployed applications and litepod-network were kept.'
	fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
	main "$@"
fi
