#!/usr/bin/env bash
# Capture the real Askama-rendered fixture reader without running an HTTP server.
# Firefox keeps its normal security sandbox enabled.
set -euo pipefail
repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
capture_browser="${CAPTURE_BROWSER:-firefox}"
if ! command -v "$capture_browser" >/dev/null 2>&1; then
  echo "Firefox is required. Set CAPTURE_BROWSER to its executable path." >&2
  exit 1
fi
WAGWAN_UI_ARTIFACT_DIR="$repo_root/.build/ui" \
  cargo test --offline web::tests::populated_story_routes_and_native_forms_work_without_sockets
mkdir -p "$repo_root/docs/screenshots" "$repo_root/.build/capture-profile"
for capture_page in story for-you discover settings; do
  capture_output="$repo_root/docs/screenshots/$capture_page.png"
  "$capture_browser" --headless --profile "$repo_root/.build/capture-profile" \
    --window-size 390,844 --screenshot "$capture_output" \
    "file://$repo_root/.build/ui/$capture_page.html"
  if [[ ! -s "$capture_output" ]]; then
    echo "Browser did not create $capture_output. Rendered HTML remains in .build/ui/." >&2
    exit 1
  fi
  echo "Captured $capture_output"
done
