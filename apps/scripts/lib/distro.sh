#!/usr/bin/env bash
# Shared distro-family detection and package-manager dispatch for litepod's
# local installers.
#
# install.sh and install-cloud.sh are distributed standalone via
# `curl | bash` and can't `source` a sibling file, so they each carry an
# inline copy of the same four functions below, kept byte-for-byte in sync
# with this file — THIS file is the canonical, tested copy. When adding a
# distro family or a package: edit here first, run test-distro.sh, then copy
# the same four function bodies verbatim into install.sh and install-cloud.sh
# (search both for "BEGIN distro.sh sync"). install-staging.sh (always run
# from a git clone) sources this file directly instead of carrying a copy.

# distro_family: prints debian|fedora|arch to stdout and returns 0 on a
# match; prints nothing and returns 1 if the host's distro isn't recognized.
# Reads /etc/os-release (or $DISTRO_OS_RELEASE_FILE, a test hook), matching
# ID= first and falling back to ID_LIKE= tokens for derivatives (e.g.
# Pop!_OS -> debian, Nobara -> fedora, Manjaro/EndeavourOS -> arch).
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

# pkg_name <family> <logical-name>: resolves one logical package name to its
# real package name on <family>. To add a package, add one case arm per
# family below. Empty output (exit 0) means the package needs no separate
# install on that family (already present in the base system).
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

# pkg_refresh <family> <run_as_root_fn>: refreshes the package manager's
# index. <run_as_root_fn> is a function name invoked as "${run}" <argv...>,
# matching the as_root_run/run_direct convention used by the callers.
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

# pkg_install <family> <run_as_root_fn> <logical-name>...: resolves every
# logical name via pkg_name, drops packages that need no separate install,
# and installs the rest in one native package-manager invocation.
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
