#!/usr/bin/env bash
set -euo pipefail

: "${S3_ENDPOINT:?S3_ENDPOINT is required}"
: "${S3_BUCKET:?S3_BUCKET is required}"
: "${S3_ACCESS_KEY:?S3_ACCESS_KEY is required}"
: "${S3_SECRET_KEY:?S3_SECRET_KEY is required}"

curl_bin="${CURL_BIN:-curl}"

for asset in install.sh uninstall.sh; do
	source_path="scripts/${asset}"
	if [[ ! -f "${source_path}" ]]; then
		printf 'Missing lifecycle script: %s\n' "${source_path}" >&2
		exit 1
	fi
	url="${S3_ENDPOINT}/${S3_BUCKET}/${asset}"
	"${curl_bin}" -fsS -X PUT \
		--aws-sigv4 "aws:amz:auto:s3" \
		--user "${S3_ACCESS_KEY}:${S3_SECRET_KEY}" \
		-H "Content-Type: text/x-shellscript" \
		--data-binary "@${source_path}" \
		"${url}" >/dev/null
done

printf 'published install.sh and uninstall.sh at %s/%s/\n' "${S3_ENDPOINT}" "${S3_BUCKET}"
