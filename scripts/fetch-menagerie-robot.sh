#!/usr/bin/env bash
# Fetch one robot folder from Google DeepMind MuJoCo Menagerie into
#   python/models/external/<robot>/
#
# Usage:
#   ./scripts/fetch-menagerie-robot.sh franka_emika_panda
#   ./scripts/fetch-menagerie-robot.sh unitree_go2
#   ./scripts/fetch-menagerie-robot.sh --list
#
# Requires: git
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST_ROOT="$ROOT/python/models/external"
MENAGERIE_URL="${MENAGERIE_URL:-https://github.com/google-deepmind/mujoco_menagerie.git}"

# Short names → upstream folder names
declare -A ALIASES=(
  [franka]=franka_emika_panda
  [franka_panda]=franka_emika_panda
  [panda]=franka_emika_panda
  [go2]=unitree_go2
  [unitree_go2]=unitree_go2
  [aloha]=aloha
  [ur5e]=universal_robots_ur5e
  [ur5]=universal_robots_ur5e
  [so100]=trs_so_arm100
  [so_arm100]=trs_so_arm100
)

list_common() {
  cat <<'EOF'
Common Menagerie robots (pass folder name or alias):

  alias / folder
  -------------
  franka / franka_panda / panda  →  franka_emika_panda
  go2 / unitree_go2              →  unitree_go2
  aloha                          →  aloha
  ur5e / ur5                     →  universal_robots_ur5e
  so100 / so_arm100              →  trs_so_arm100

Full list: https://github.com/google-deepmind/mujoco_menagerie
After fetch: --model builtin:<catalog_name>  or  --model python/models/external/<folder>/scene.xml
EOF
}

if [[ "${1:-}" == "" || "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  echo "Usage: $0 <robot_folder_or_alias>"
  echo "       $0 --list"
  exit 0
fi

if [[ "${1}" == "--list" ]]; then
  list_common
  exit 0
fi

KEY="$1"
ROBOT="${ALIASES[$KEY]:-$KEY}"
OUT="$DEST_ROOT/$ROBOT"

if [[ -f "$OUT/scene.xml" || -f "$OUT/mjx_scene.xml" ]]; then
  echo "[ok] already present: $OUT"
  ls "$OUT"/*.xml 2>/dev/null | head -5
  exit 0
fi

mkdir -p "$DEST_ROOT"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/archon-menagerie.XXXXXX")"
cleanup() { rm -rf "$TMP"; }
trap cleanup EXIT

echo "[fetch] sparse-clone $ROBOT from mujoco_menagerie …"
git clone --depth 1 --filter=blob:none --sparse "$MENAGERIE_URL" "$TMP/menagerie"
git -C "$TMP/menagerie" sparse-checkout set "$ROBOT"

if [[ ! -d "$TMP/menagerie/$ROBOT" ]]; then
  echo "error: folder '$ROBOT' not found in Menagerie. Try: $0 --list" >&2
  exit 1
fi

rm -rf "$OUT"
mv "$TMP/menagerie/$ROBOT" "$OUT"

echo "[ok] installed → $OUT"
if [[ -f "$OUT/scene.xml" ]]; then
  echo "run: cargo run -p archon-embodied-cli -- --backend mujoco --model $OUT/scene.xml --viewer --step-ms 0"
elif [[ -f "$OUT/mjx_scene.xml" ]]; then
  echo "run: cargo run -p archon-embodied-cli -- --backend mujoco --model $OUT/mjx_scene.xml --viewer --step-ms 0"
else
  echo "note: no scene.xml; pick an xml under $OUT"
  ls "$OUT"/*.xml 2>/dev/null | head -10 || true
fi
