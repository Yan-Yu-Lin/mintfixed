#!/usr/bin/env bash
# Replace MintCat's integration in a Linux/Proton DRG install with mint's.
#
# Run with the game AND MintCat closed. Steps:
#   0. record the sha256 of every file in ue4ss/mods/ (unless a list from an earlier backup is given)
#   1. move away what MintCat's uninstall would delete (its merged pak, its audio paks, an old
#      mods_P.pak, the x3daudio1_7 hook, dwmapi.dll and the whole ue4ss/ tree) into $MOVED_TO --
#      nothing is deleted
#   2. integrate the given mint profile with the mint CLI
#   3. copy each mod's old runtime files (logs, configs: everything outside js/ that is not a .dll),
#      UE4SS.log and UE4SS-settings.ini back (only where the mod folder exists again, never
#      overwriting)
#   4. verify: every file that was in ue4ss/mods/ (except logs) must be back byte-identical
#
# Usage: switch-from-mintcat.sh <backup.sha256|auto> [profile] [mint-binary]
#   auto            take the sha256 list from the game dir right before moving (step 0)
#   MINT_APPDATA    pass --appdata to mint (rehearsals with an isolated mint config)
# Exit: 0 ok, 1 failed (MintCat's files are in $MOVED_TO), 2 integrated but verification differs
set -euo pipefail

SHA_LIST=${1:?usage: $0 <backup.sha256|auto> [profile] [mint-binary]}
PROFILE=${2:-"MintCat 新的默認"}
MINT=${3:-mintfixed}
GAME=${DRG_DIR:-"/mnt/data/SteamLibrary/steamapps/common/Deep Rock Galactic"}
WIN64="$GAME/FSD/Binaries/Win64"
PAKS="$GAME/FSD/Content/Paks"
MOVED_TO=${MOVED_TO:-"/mnt/data/drgmod/backups/mintcat-removed-$(date +%Y%m%d-%H%M%S)"}
MINT_ARGS=()
[[ -n "${MINT_APPDATA:-}" ]] && MINT_ARGS=(--appdata "$MINT_APPDATA")

# SKIP_PROCESS_CHECK=1 is only for rehearsing on a copy of the game (DRG_DIR pointing elsewhere)
if [[ -z "${SKIP_PROCESS_CHECK:-}" ]]; then
  if pgrep -f FSD-Win64-Shipping >/dev/null; then echo "DRG is running, close it first" >&2; exit 1; fi
  if pgrep -x mintcat >/dev/null || pgrep -f '/mintcat( |$)' >/dev/null; then
    echo "MintCat is running, close it first" >&2; exit 1
  fi
fi
[[ -e "$MOVED_TO" ]] && { echo "$MOVED_TO already exists" >&2; exit 1; }
mkdir -p "$MOVED_TO/Paks" "$MOVED_TO/Win64"

if [[ "$SHA_LIST" == auto ]]; then
  SHA_LIST="$MOVED_TO/before.sha256"
  echo "== 0. recording ue4ss/mods checksums in $SHA_LIST"
  if [[ -d "$WIN64/ue4ss/mods" ]]; then
    (cd "$GAME" && find FSD/Binaries/Win64/ue4ss/mods -type f -print0 | sort -z | xargs -0 -r sha256sum) >"$SHA_LIST"
  else
    : >"$SHA_LIST"
  fi
fi
[[ -f "$SHA_LIST" ]] || { echo "missing backup list $SHA_LIST" >&2; exit 1; }

echo "== 1. moving MintCat's integration to $MOVED_TO"
shopt -s nullglob
for f in "$PAKS/FSD-WindowsNoEditor_Mods.pak" "$PAKS/mods_P.pak" "$PAKS"/*_audio_*_P.pak; do
  if [[ -e "$f" ]]; then mv -- "$f" "$MOVED_TO/Paks/"; fi
done
shopt -u nullglob
for f in "$WIN64/x3daudio1_7.dll" "$WIN64/dwmapi.dll" "$WIN64/ue4ss"; do
  if [[ -e "$f" ]]; then mv -- "$f" "$MOVED_TO/Win64/"; fi
done

echo "== 2. mint integrate: $PROFILE"
if ! "$MINT" "${MINT_ARGS[@]}" profile "$PROFILE" --fsd-pak "$PAKS/FSD-WindowsNoEditor.pak"; then
  echo "mint integration failed. MintCat's files are in $MOVED_TO; to put them back:" >&2
  echo "  cp -a \"$MOVED_TO/Paks/.\" \"$PAKS/\" && cp -a \"$MOVED_TO/Win64/.\" \"$WIN64/\"" >&2
  exit 1
fi

echo "== 3. restoring per-mod logs/config and UE4SS settings"
# runtime files a mod wrote into its folder (logs, configs); code (js/, *.dll) comes from mint
if [[ -d "$MOVED_TO/Win64/ue4ss/mods" ]]; then
  for d in "$MOVED_TO/Win64/ue4ss/mods"/*/; do
    name=$(basename "$d")
    [[ -d "$WIN64/ue4ss/mods/$name" ]] || continue
    (cd "$d" && find . -type f -not -path './js/*' -not -iname '*.dll' -print0) |
      while IFS= read -r -d '' rel; do
        dest="$WIN64/ue4ss/mods/$name/${rel#./}"
        [[ -e "$dest" ]] && continue
        mkdir -p -- "$(dirname -- "$dest")"
        cp -p -- "$d/${rel#./}" "$dest" && echo "restored ue4ss/mods/$name/${rel#./}"
      done
  done
fi
if [[ -d "$WIN64/ue4ss" ]]; then
  for f in UE4SS.log UE4SS-settings.ini; do
    if [[ -f "$MOVED_TO/Win64/ue4ss/$f" ]]; then cp -n -- "$MOVED_TO/Win64/ue4ss/$f" "$WIN64/ue4ss/"; fi
  done
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
expected=$(grep -E 'ue4ss/mods/' "$SHA_LIST" | grep -vE '\.log$|/\.mint-managed\.json$' || true)
n=0
while read -r sum path; do
  [[ -z "$path" ]] && continue
  n=$((n + 1))
  actual=$(sha256sum "$GAME/$path" 2>/dev/null | cut -d' ' -f1 || true)
  [[ "$actual" == "$sum" ]] || { echo "FAIL $path ${actual:-missing}"; fail=1; }
done <<<"$expected"
echo "compared $n ue4ss/mods files against the backup"
echo "mod folders now: $(find "$WIN64/ue4ss/mods" -mindepth 1 -maxdepth 1 -type d 2>/dev/null | wc -l)"
exit $((fail * 2))
