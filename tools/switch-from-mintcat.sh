#!/usr/bin/env bash
# Replace MintCat's integration in a Linux/Proton DRG install with mint's.
#
# Run with the game AND MintCat closed. Steps:
#   1. move away what MintCat's uninstall would delete (its merged pak, an old mods_P.pak, the
#      x3daudio1_7 hook, dwmapi.dll and the whole ue4ss/ tree) into $MOVED_TO -- nothing is deleted
#   2. integrate the given mint profile with the mint CLI
#   3. copy each mod's old *.log files back (only where the mod folder exists again)
#   4. verify: every file that was in ue4ss/mods/ (except logs) must be back byte-identical,
#      compared against the sha256 list written when the backup was taken
#
# Usage: switch-from-mintcat.sh <backup.sha256> [profile] [mint-binary]
set -euo pipefail

SHA_LIST=${1:?usage: $0 <backup.sha256> [profile] [mint-binary]}
PROFILE=${2:-"MintCat 新的默認"}
MINT=${3:-mintfixed}
GAME=${DRG_DIR:-"/mnt/data/SteamLibrary/steamapps/common/Deep Rock Galactic"}
WIN64="$GAME/FSD/Binaries/Win64"
PAKS="$GAME/FSD/Content/Paks"
MOVED_TO=${MOVED_TO:-"/mnt/data/drgmod/backups/mintcat-removed-$(date +%Y%m%d-%H%M%S)"}

# SKIP_PROCESS_CHECK=1 is only for rehearsing on a copy of the game (DRG_DIR pointing elsewhere)
if [[ -z "${SKIP_PROCESS_CHECK:-}" ]]; then
  if pgrep -f FSD-Win64-Shipping >/dev/null; then echo "DRG is running, close it first" >&2; exit 1; fi
  if pgrep -x mintcat >/dev/null || pgrep -f '/mintcat( |$)' >/dev/null; then
    echo "MintCat is running, close it first" >&2; exit 1
  fi
fi
[[ -f "$SHA_LIST" ]] || { echo "missing backup list $SHA_LIST" >&2; exit 1; }

echo "== 1. moving MintCat's integration to $MOVED_TO"
mkdir -p "$MOVED_TO/Paks" "$MOVED_TO/Win64"
for f in "$PAKS/FSD-WindowsNoEditor_Mods.pak" "$PAKS/mods_P.pak"; do
  if [[ -e "$f" ]]; then mv -- "$f" "$MOVED_TO/Paks/"; fi
done
for f in "$WIN64/x3daudio1_7.dll" "$WIN64/dwmapi.dll" "$WIN64/ue4ss"; do
  if [[ -e "$f" ]]; then mv -- "$f" "$MOVED_TO/Win64/"; fi
done

echo "== 2. mint integrate: $PROFILE"
if ! "$MINT" profile "$PROFILE" --fsd-pak "$PAKS/FSD-WindowsNoEditor.pak"; then
  echo "mint integration failed. MintCat's files are in $MOVED_TO; to put them back:" >&2
  echo "  cp -a \"$MOVED_TO/Paks/.\" \"$PAKS/\" && cp -a \"$MOVED_TO/Win64/.\" \"$WIN64/\"" >&2
  exit 1
fi

echo "== 3. restoring per-mod logs"
if [[ -d "$MOVED_TO/Win64/ue4ss/mods" ]]; then
  for d in "$MOVED_TO/Win64/ue4ss/mods"/*/; do
    name=$(basename "$d")
    [[ -d "$WIN64/ue4ss/mods/$name" ]] || continue
    find "$d" -maxdepth 1 -type f -name '*.log' -exec cp -n -- {} "$WIN64/ue4ss/mods/$name/" \;
  done
  if [[ -f "$MOVED_TO/Win64/ue4ss/UE4SS.log" ]]; then
    cp -n -- "$MOVED_TO/Win64/ue4ss/UE4SS.log" "$WIN64/ue4ss/"
  fi
fi

echo "== 4. verify"
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
check "mods_P.pak present"                     '[[ -s "$PAKS/mods_P.pak" ]]'
check "FSD-WindowsNoEditor_Mods.pak gone"      '[[ ! -e "$PAKS/FSD-WindowsNoEditor_Mods.pak" ]]'
check "x3daudio1_7.dll (mint hook) present"    '[[ -s "$WIN64/x3daudio1_7.dll" ]]'
check "dwmapi.dll (UE4SSL proxy) present"      '[[ -s "$WIN64/dwmapi.dll" ]]'
check "ue4ss/UE4SSL.dll present"               '[[ -s "$WIN64/ue4ss/UE4SSL.dll" ]]'
check "mint manifest present"                  '[[ -s "$WIN64/ue4ss/mods/.mint-managed.json" ]]'
# every file that was in a ue4ss/mods/<mod>/ folder before (except logs) must be back, byte-identical
expected=$(grep -E 'ue4ss/mods/' "$SHA_LIST" | grep -vE '\.log$' || true)
n=0
while read -r sum path; do
  [[ -z "$path" ]] && continue
  n=$((n + 1))
  actual=$(sha256sum "$GAME/$path" 2>/dev/null | cut -d' ' -f1 || true)
  [[ "$actual" == "$sum" ]] || { echo "FAIL $path ${actual:-missing}"; fail=1; }
done <<<"$expected"
echo "compared $n ue4ss/mods files against the backup"
echo "mod folders now: $(find "$WIN64/ue4ss/mods" -mindepth 1 -maxdepth 1 -type d | wc -l)"
exit $fail
