#!/usr/bin/env bash
set -euo pipefail

: "${IMAGE:?IMAGE is required}"
: "${REPOSITORY:?REPOSITORY is required}"
: "${TAG:?TAG is required}"

max_attempts="${MAX_ATTEMPTS:-24}"
retry_delay_secs="${RETRY_DELAY_SECS:-5}"
sleep_bin="${SLEEP_BIN:-sleep}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [[ ! "${max_attempts}" =~ ^[1-9][0-9]*$ ]]; then
	printf '%s\n' "MAX_ATTEMPTS must be a positive integer (got ${max_attempts})." >&2
	exit 1
fi
if [[ ! "${retry_delay_secs}" =~ ^[0-9]+$ ]]; then
	printf '%s\n' "RETRY_DELAY_SECS must be a non-negative integer (got ${retry_delay_secs})." >&2
	exit 1
fi

for ((attempt = 1; attempt <= max_attempts; attempt++)); do
	if ! state="$(IMAGE="${IMAGE}" REPOSITORY="${REPOSITORY}" TAG="${TAG}" \
		CURL_BIN="${CURL_BIN:-curl}" bash "${script_dir}/check-immutable-tag.sh")"; then
		exit 1
	fi
	case "${state}" in
	present)
		printf '%s\n' "immutable tag ${TAG} is resolvable through the registry"
		exit 0
		;;
	absent)
		if (( attempt == max_attempts )); then
			break
		fi
		printf '%s\n' "waiting for immutable tag ${TAG} to appear in the registry (${attempt}/${max_attempts})"
		"${sleep_bin}" "${retry_delay_secs}"
		;;
	*)
		printf '%s\n' "Unexpected immutable tag state for ${IMAGE}: ${state}." >&2
		exit 1
		;;
	esac
done

printf '%s\n' "Immutable tag ${TAG} was not resolvable through the registry after ${max_attempts} attempts." >&2
exit 1
