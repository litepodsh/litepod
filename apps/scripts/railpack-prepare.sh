#!/bin/sh
# Runs with access to the application data volume, but only generates a
# Railpack plan. The actual build runs later in an unprivileged container with
# no /data mount.
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
