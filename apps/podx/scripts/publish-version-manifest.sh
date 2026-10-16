#!/usr/bin/env bash
set -euo pipefail

: "${S3_ACCESS_KEY:?S3_ACCESS_KEY is required}"
: "${S3_SECRET_KEY:?S3_SECRET_KEY is required}"
: "${URL:?URL is required}"

curl_bin="${CURL_BIN:-curl}"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest_merger="${MANIFEST_MERGER:-${script_dir}/merge-version-manifest.py}"
if [[ ! -f "${manifest_merger}" ]]; then
	printf 'Manifest merger is unavailable: %s\n' "${manifest_merger}" >&2
	exit 1
fi
if [[ -z "${MANIFEST_MERGER:-}" ]]; then
	: "${VERSION:?VERSION is required}"
	: "${CHANNEL:?CHANNEL is required}"
	if [[ "${CHANNEL}" != stable && "${CHANNEL}" != alpha ]]; then
		printf '%s\n' 'CHANNEL must be stable or alpha' >&2
		exit 1
	fi
	publication_label="${VERSION} (${CHANNEL})"
else
	publication_label="core services"
fi
scratch="$(mktemp -d)"
trap 'rm -rf -- "${scratch}"' EXIT

for attempt in 1 2 3 4 5; do
	headers="${scratch}/get-headers"
	current="${scratch}/current.json"
	response="${scratch}/put-response"

	if ! get_status="$("${curl_bin}" -sS -D "${headers}" -o "${current}" \
		-w '%{http_code}' "${URL}")"; then
		printf '%s\n' "Could not read release manifest at ${URL}." >&2
		exit 1
	fi

	condition_header=''
	case "${get_status}" in
	200)
		etag="$(awk '/^[Ee][Tt][Aa][Gg]:/ { sub(/\r$/, ""); sub(/^[^:]*:[[:space:]]*/, ""); print; exit }' "${headers}")"
		if [[ -z "${etag}" ]]; then
			printf '%s\n' "Release manifest GET returned 200 without an ETag; refusing an unconditional write." >&2
			exit 1
		fi
		condition_header="If-Match: ${etag}"
		;;
	404)
		printf '%s' '{}' > "${current}"
		condition_header='If-None-Match: *'
		;;
	*)
		printf '%s\n' "Could not read release manifest at ${URL}: HTTP ${get_status}." >&2
		exit 1
		;;
	esac

	python3 "${manifest_merger}" < "${current}" > "${scratch}/next.json"

	if ! put_status="$("${curl_bin}" -sS -o "${response}" -w '%{http_code}' -X PUT \
		--aws-sigv4 "aws:amz:auto:s3" \
		--user "${S3_ACCESS_KEY}:${S3_SECRET_KEY}" \
		-H "Content-Type: application/json" \
		-H "${condition_header}" \
		--data-binary "@${scratch}/next.json" \
		"${URL}")"; then
		printf '%s\n' "Could not write release manifest at ${URL}." >&2
		exit 1
	fi

	case "${put_status}" in
	200|201|204)
		printf 'published %s at %s\n' "${publication_label}" "${URL}"
		exit 0
		;;
	409|412)
		if (( attempt < 5 )); then
			continue
		fi
		printf '%s\n' 'Release manifest changed repeatedly during publication; retry the release.' >&2
		exit 1
		;;
	*)
		printf '%s\n' "Could not write release manifest at ${URL}: HTTP ${put_status}." >&2
		exit 1
		;;
	esac
done
