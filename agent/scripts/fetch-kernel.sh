#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
LOCK_FILE="${1:-$ROOT_DIR/kernel.lock}"
DEST_DIR="${KERNEL_DEST_DIR:-$ROOT_DIR/.cache/kernels}"

if [[ ! -f "$LOCK_FILE" ]]; then
  echo "Lock file not found: $LOCK_FILE" >&2
  exit 1
fi

KERNEL_URL=""
KERNEL_SHA256=""
KERNEL_FILENAME=""

while IFS='=' read -r key raw_value; do
  [[ -z "${key// }" ]] && continue
  [[ "${key#"${key%%[![:space:]]*}"}" == \#* ]] && continue

  key="${key//[[:space:]]/}"
  value="${raw_value#"${raw_value%%[![:space:]]*}"}"
  value="${value%"${value##*[![:space:]]}"}"
  value="${value%\"}"
  value="${value#\"}"

  case "$key" in
    KERNEL_URL) KERNEL_URL="$value" ;;
    KERNEL_SHA256) KERNEL_SHA256="$value" ;;
    KERNEL_FILENAME) KERNEL_FILENAME="$value" ;;
  esac
done < "$LOCK_FILE"

if [[ -z "$KERNEL_URL" || -z "$KERNEL_SHA256" ]]; then
  echo "Invalid lock file: KERNEL_URL and KERNEL_SHA256 are required" >&2
  exit 1
fi

mkdir -p "$DEST_DIR"

filename="${KERNEL_FILENAME:-$(basename "$KERNEL_URL")}"
dest="$DEST_DIR/$filename"

tmp="$(mktemp "${dest}.tmp.XXXXXX")"
cleanup_tmp() { rm -f "$tmp"; }
trap cleanup_tmp EXIT

echo "Downloading kernel from $KERNEL_URL"
curl -fL --retry 3 --retry-delay 1 --connect-timeout 10 --max-time 300 -o "$tmp" "$KERNEL_URL"

echo "Verifying checksum..."
echo "$KERNEL_SHA256  $tmp" | sha256sum -c -

mv "$tmp" "$dest"
trap - EXIT

echo "Kernel installed at: $dest"
echo "Use it with: AGENT_KERNEL_PATH=$dest cargo run -p agent"

