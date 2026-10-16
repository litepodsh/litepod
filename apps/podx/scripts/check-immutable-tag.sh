#!/usr/bin/env bash
set -euo pipefail

: "${IMAGE:?IMAGE is required}"
: "${REPOSITORY:?REPOSITORY is required}"
: "${TAG:?TAG is required}"

curl_bin="${CURL_BIN:-curl}"
body="$(mktemp)"
trap 'rm -f -- "${body}"' EXIT

token_url="https://auth.docker.io/token?service=registry.docker.io&scope=repository:${REPOSITORY}:pull"
if ! token_status="$("${curl_bin}" -sS -o "${body}" -w '%{http_code}' "${token_url}")"; then
	printf '%s\n' "Could not request a registry token for ${IMAGE}." >&2
	exit 1
fi
if [[ "${token_status}" != 200 ]]; then
	printf '%s\n' "Could not request a registry token for ${IMAGE}: HTTP ${token_status}." >&2
	exit 1
fi

token="$(python3 -c '
import json, sys
try:
    value = json.load(open(sys.argv[1]))
    token = value.get("token") or value.get("access_token") or ""
except (OSError, ValueError, AttributeError):
    token = ""
print(token)
' "${body}")"
if [[ -z "${token}" ]]; then
	printf '%s\n' "Registry token response for ${IMAGE} did not contain a token." >&2
	exit 1
fi

manifest_url="https://registry-1.docker.io/v2/${REPOSITORY}/manifests/${TAG}"
accept='application/vnd.oci.image.index.v1+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.docker.distribution.manifest.v2+json'
if ! status="$("${curl_bin}" -sS -H "Authorization: Bearer ${token}" -H "Accept: ${accept}" -o "${body}" -w '%{http_code}' "${manifest_url}")"; then
	printf '%s\n' "Could not resolve immutable tag ${IMAGE} through the registry." >&2
	exit 1
fi

case "${status}" in
404)
	printf '%s\n' absent
	exit 0
	;;
200) ;;
*)
	printf '%s\n' "Could not resolve immutable tag ${IMAGE} through the registry: HTTP ${status}." >&2
	exit 1
	;;
esac

schema_version="$(python3 -c '
import json, sys
try:
    value = json.load(open(sys.argv[1])).get("schemaVersion")
except (OSError, ValueError, AttributeError):
    value = None
print(value)
' "${body}")"
if [[ "${schema_version}" != 2 ]]; then
	printf '%s\n' "Registry returned an invalid manifest for ${IMAGE}." >&2
	exit 1
fi

printf '%s\n' present
