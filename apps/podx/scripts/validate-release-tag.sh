#!/usr/bin/env bash
set -euo pipefail

channel="${1:-}"
tag="${2:-}"

case "${channel}" in
stable)
	if [[ ! "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
		printf 'stable release tag must match vX.Y.Z (got %s)\n' "${tag:-empty}" >&2
		exit 1
	fi
	;;
alpha)
	if [[ ! "${tag}" =~ ^v[0-9]+\.[0-9]+\.[0-9]+-alpha\.[0-9]+$ ]]; then
		printf 'alpha release tag must match vX.Y.Z-alpha.N (got %s)\n' "${tag:-empty}" >&2
		exit 1
	fi
	;;
*)
	printf 'channel must be stable or alpha (got %s)\n' "${channel:-empty}" >&2
	exit 1
	;;
esac
