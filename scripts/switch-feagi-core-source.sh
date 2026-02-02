#!/usr/bin/env bash
set -euo pipefail

MODE="${1:-}"
ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
CORE_DIR="${ROOT_DIR}/../feagi-core"
CONFIG_DIR="${ROOT_DIR}/.cargo"
CONFIG_FILE="${CONFIG_DIR}/config.toml"
CONFIG_TEMPLATE="${CONFIG_DIR}/config.toml.example"

if [ "$MODE" != "local" ] && [ "$MODE" != "remote" ]; then
  echo "Usage: $(basename "$0") [local|remote]"
  exit 1
fi

if [ ! -d "$CORE_DIR" ]; then
  echo "Missing feagi-core at: $CORE_DIR"
  exit 1
fi

get_workspace_version() {
  local workspace_toml="$CORE_DIR/Cargo.toml"
  awk '
    BEGIN { in_ws=0 }
    /^\[workspace\.package\]/ { in_ws=1; next }
    /^\[/ { if (in_ws==1) exit }
    in_ws==1 && /^version = / {
      gsub(/version = /, "", $0);
      gsub(/"/, "", $0);
      print $0;
      exit
    }
  ' "$workspace_toml"
}

get_local_version() {
  local crate_name="$1"
  local crate_path="$2"
  local manifest="${CORE_DIR}/${crate_path}/Cargo.toml"
  if [ ! -f "$manifest" ]; then
    echo "Missing manifest for ${crate_name} at ${manifest}"
    exit 1
  fi

  if grep -q '^version\.workspace = true' "$manifest"; then
    get_workspace_version
  else
    grep '^version = ' "$manifest" | head -1 | sed 's/version = "\(.*\)"/\1/'
  fi
}

get_remote_version() {
  local crate_name="$1"
  local result
  result="$(cargo search "$crate_name" --limit 1 2>/dev/null | grep "^$crate_name = " | sed 's/.*"\(.*\)".*/\1/' || true)"
  if [ -z "$result" ]; then
    echo "Failed to resolve remote version for ${crate_name}"
    exit 1
  fi
  echo "$result"
}

update_cargo_toml() {
  local target_versions_json="$1"
  VERSIONS_JSON="$target_versions_json" ROOT_DIR="$ROOT_DIR" python3 - <<'PY'
import json
import os
import re
from pathlib import Path

root_dir = Path(os.environ["ROOT_DIR"])
cargo_toml = root_dir / "Cargo.toml"
data = cargo_toml.read_text(encoding="utf-8")
versions = json.loads(os.environ["VERSIONS_JSON"])

for name, version in versions.items():
    pattern = rf'^({re.escape(name)}\s*=\s*\{{[^}}]*version\s*=\s*")[^"]+(")'
    replacement = rf'\g<1>{version}\g<2>'
    data, count = re.subn(pattern, replacement, data, flags=re.MULTILINE)
    if count == 0:
        raise SystemExit(f"Failed to update version for {name} in Cargo.toml")

cargo_toml.write_text(data, encoding="utf-8")
PY
}

CRATES=(
  "feagi-npu-neural:crates/feagi-npu/neural"
  "feagi-structures:crates/feagi-structures"
  "feagi-npu-runtime:crates/feagi-npu/runtime"
  "feagi-config:crates/feagi-config"
  "feagi-npu-burst-engine:crates/feagi-npu/burst-engine"
  "feagi-brain-development:crates/feagi-brain-development"
  "feagi-evolutionary:crates/feagi-evolutionary"
  "feagi-services:crates/feagi-services"
  "feagi-api:crates/feagi-api"
  "feagi-io:crates/feagi-io"
  "feagi-state-manager:crates/feagi-state-manager"
  "feagi-npu-plasticity:crates/feagi-npu/plasticity"
  "feagi-observability:crates/feagi-observability"
)

VERSIONS_JSON="{"
for entry in "${CRATES[@]}"; do
  crate_name="${entry%%:*}"
  crate_path="${entry#*:}"
  if [ "$MODE" = "local" ]; then
    version="$(get_local_version "$crate_name" "$crate_path")"
  else
    version="$(get_remote_version "$crate_name")"
  fi
  if [ -n "$VERSIONS_JSON" ] && [ "$VERSIONS_JSON" != "{" ]; then
    VERSIONS_JSON+=","
  fi
  VERSIONS_JSON+="\"${crate_name}\":\"${version}\""
done
VERSIONS_JSON+="}"

update_cargo_toml "${VERSIONS_JSON}"

if [ "$MODE" = "local" ]; then
  mkdir -p "$CONFIG_DIR"
  if [ -f "$CONFIG_TEMPLATE" ]; then
    cp "$CONFIG_TEMPLATE" "$CONFIG_FILE"
  else
    echo "[patch.crates-io]" > "$CONFIG_FILE"
    for entry in "${CRATES[@]}"; do
      crate_name="${entry%%:*}"
      crate_path="${entry#*:}"
      echo "${crate_name} = { path = \"../feagi-core/${crate_path}\" }" >> "$CONFIG_FILE"
    done
  fi
  echo "Switched to local feagi-core via ${CONFIG_FILE}"
else
  if [ -f "$CONFIG_FILE" ]; then
    mv "$CONFIG_FILE" "${CONFIG_FILE}.disabled"
    echo "Disabled local patches: ${CONFIG_FILE}.disabled"
  fi
  echo "Switched to crates.io versions"
fi

(
  cd "$ROOT_DIR"
  cargo generate-lockfile
)
