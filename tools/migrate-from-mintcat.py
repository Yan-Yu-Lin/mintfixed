#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Copy a MintCat profile into mint's config.

Reads MintCat's sqlite database (read-only snapshot, the file is never modified) and writes
mint's `mod_data.json` and `config.json`:

- the MintCat profile becomes a mint profile (default name "MintCat <profile name>") and is made
  the active profile; any other mint profiles are kept,
- every MintCat folder becomes a mint group named "MintCat: <folder path>", in MintCat's order;
  mods keep their enabled/disabled state (local mods by file path, mod.io mods by URL),
- the mod.io OAuth token, the DRG pak path and the UE4SSL.zip path are set in config.json,
- the SHA-256 of the native DLL inside every *enabled local* mod is added to
  `confirmed_native_dlls`, so installing them does not ask for confirmation again.

Existing mint files are backed up next to themselves before being replaced. Re-running replaces
the migrated profile and its groups and leaves everything else alone.

    uv run tools/migrate-from-mintcat.py --dry-run
    uv run tools/migrate-from-mintcat.py --profile-id 2
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sqlite3
import sys
import time
import zipfile
from pathlib import Path

GROUP_PREFIX = "MintCat: "
RESERVED_DLLS = {"dwmapi.dll", "ue4ssl.dll"}


def xdg(var: str, default: str) -> Path:
    return Path(os.environ.get(var) or Path.home() / default)


def default_paths() -> tuple[Path, Path, Path]:
    db = xdg("XDG_CONFIG_HOME", ".config") / "com.mint.cat" / "mintcat.sqlite"
    mint_config = xdg("XDG_CONFIG_HOME", ".config") / "mint"
    ue4ssl = xdg("XDG_CACHE_HOME", ".cache") / "com.mint.cat" / "UE4SSL.zip"
    return db, mint_config, ue4ssl


def snapshot(db: Path) -> sqlite3.Connection:
    """Consistent in-memory copy of the database, opened read-only."""
    src = sqlite3.connect(f"file:{db}?mode=ro", uri=True)
    mem = sqlite3.connect(":memory:")
    src.backup(mem)
    src.close()
    mem.row_factory = sqlite3.Row
    return mem


def native_dll_sha256(path: Path) -> str | None:
    """Same rule as mint: dll/main.dll, otherwise the first non-loader .dll in the zip."""
    try:
        zf = zipfile.ZipFile(path)
    except (zipfile.BadZipFile, OSError):
        return None
    with zf:
        names = [i.filename for i in zf.infolist() if not i.is_dir()]
        lower = [n.replace("\\", "/").lower() for n in names]
        pick = None
        for name, low in zip(names, lower):
            parts = [p for p in low.split("/") if p]
            if parts[-1:] == ["main.dll"] and parts[-2:-1] == ["dll"]:
                pick = name
                break
        if pick is None:
            for name, low in zip(names, lower):
                base = low.rsplit("/", 1)[-1]
                if base.endswith(".dll") and base not in RESERVED_DLLS:
                    pick = name
                    break
        if pick is None:
            return None
        return hashlib.sha256(zf.read(pick)).hexdigest()


def load_profile(con: sqlite3.Connection, profile_id: int):
    profile = con.execute(
        "select id, name, display_name from profiles where id = ?", (profile_id,)
    ).fetchone()
    if profile is None:
        sys.exit(f"MintCat profile {profile_id} not found")
    folders = {
        r["id"]: dict(r)
        for r in con.execute(
            "select id, parent_folder_id, name, sort_order from profile_folders where profile_id = ?",
            (profile_id,),
        )
    }
    mods = [
        dict(r)
        for r in con.execute(
            """select pm.parent_folder_id, pm.sort_order, pm.is_enabled, pm.used_version,
                      m.source_type, m.url, m.display_name, m.name_id
               from profile_mods pm join mods m using (mod_id)
               where pm.profile_id = ?
               order by pm.sort_order, pm.id""",
            (profile_id,),
        )
    ]
    oauth = con.execute(
        "select oauth from oauths where platform = 'mod.io' and oauth != '' order by id desc"
    ).fetchone()
    pak = con.execute("select install_path from games where id = 1").fetchone()
    return profile, folders, mods, oauth and oauth["oauth"], pak and pak["install_path"]


def folder_path(folders: dict, fid: int) -> str:
    parts = []
    seen = set()
    while fid is not None and fid in folders and fid not in seen:
        seen.add(fid)
        parts.append(folders[fid]["name"])
        fid = folders[fid]["parent_folder_id"]
    return " / ".join(reversed(parts))


def folder_order(folders: dict) -> list[int]:
    """Depth-first, children after their parent, siblings by MintCat sort order."""
    children: dict[int | None, list[int]] = {}
    for f in folders.values():
        children.setdefault(f["parent_folder_id"], []).append(f["id"])
    for ids in children.values():
        ids.sort(key=lambda i: (folders[i]["sort_order"], i))
    out: list[int] = []

    def walk(parent):
        for fid in children.get(parent, []):
            out.append(fid)
            walk(fid)

    walk(None)
    return out


def mod_spec(mod: dict) -> str | None:
    if mod["source_type"] == "Local":
        return mod["url"] or None
    if mod["source_type"] == "Modio":
        url = mod["url"] or (mod["name_id"] and f"https://mod.io/g/drg/m/{mod['name_id']}")
        return url or None
    return None


def backup(path: Path, stamp: str) -> None:
    if path.exists():
        dest = path.with_name(f"{path.name}.bak-{stamp}")
        dest.write_bytes(path.read_bytes())
        print(f"backed up {path} -> {dest.name}")


def main() -> None:
    db_default, cfg_default, ue4ssl_default = default_paths()
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--db", type=Path, default=db_default, help="MintCat sqlite database")
    ap.add_argument("--profile-id", type=int, default=None, help="MintCat profile id (default: the active one)")
    ap.add_argument("--name", help="mint profile name (default: 'MintCat <MintCat profile name>')")
    ap.add_argument("--config-dir", type=Path, default=cfg_default, help="mint config directory")
    ap.add_argument("--pak", help="FSD-WindowsNoEditor.pak (default: MintCat's DRG install path)")
    ap.add_argument("--ue4ssl-zip", type=Path, default=ue4ssl_default)
    ap.add_argument("--dry-run", action="store_true", help="print what would be written, write nothing")
    args = ap.parse_args()

    con = snapshot(args.db)
    profile_id = args.profile_id
    if profile_id is None:
        row = con.execute("select id from profiles where is_active = 1 order by id limit 1").fetchone()
        if row is None:
            sys.exit("no active MintCat profile; pass --profile-id")
        profile_id = row["id"]
    profile, folders, mods, oauth, mintcat_pak = load_profile(con, profile_id)
    name = args.name or f"MintCat {profile['display_name'] or profile['name']}"
    pak = args.pak or mintcat_pak

    by_folder: dict[int | None, list[dict]] = {}
    for m in mods:
        by_folder.setdefault(m["parent_folder_id"], []).append(m)

    unmapped: list[str] = []
    missing_enabled: list[str] = []
    pinned: list[str] = []
    confirmed: set[str] = set()
    native_mods: list[str] = []

    def mod_config(m: dict) -> dict | None:
        spec = mod_spec(m)
        if spec is None:
            unmapped.append(f"{m['display_name']} ({m['source_type']})")
            return None
        enabled = bool(m["is_enabled"])
        if m["used_version"] not in ("", "-"):
            pinned.append(f"{m['display_name']} @ {m['used_version']}")
        if m["source_type"] == "Local" and enabled:
            path = Path(spec)
            if not path.exists():
                missing_enabled.append(spec)
            else:
                digest = native_dll_sha256(path)
                if digest:
                    confirmed.add(digest)
                    native_mods.append(path.name)
        return {"spec": {"url": spec}, "required": False, "enabled": enabled}

    profile_entries: list[dict] = []
    groups: dict[str, dict] = {}
    for m in by_folder.get(None, []):
        mc = mod_config(m)
        if mc:
            profile_entries.append(mc)
    for fid in folder_order(folders):
        entries = [mc for m in by_folder.get(fid, []) if (mc := mod_config(m))]
        if not entries:
            continue
        group_name = GROUP_PREFIX + folder_path(folders, fid)
        groups[group_name] = {"mods": entries}
        profile_entries.append({"group_name": group_name, "enabled": True})
    for fid in set(by_folder) - set(folders) - {None}:
        for m in by_folder[fid]:
            mc = mod_config(m)
            if mc:
                profile_entries.append(mc)

    # ---- merge into existing mint files ----
    mod_data_path = args.config_dir / "mod_data.json"
    config_path = args.config_dir / "config.json"
    mod_data = (
        json.loads(mod_data_path.read_text())
        if mod_data_path.exists()
        else {"version": "0.1.0", "active_profile": name, "profiles": {}, "groups": {}}
    )
    if mod_data.get("version") != "0.1.0":
        sys.exit(f"{mod_data_path}: unsupported mod_data version {mod_data.get('version')!r}; open mint once to upgrade it")
    for g in [g for g in mod_data.get("groups", {}) if g.startswith(GROUP_PREFIX)]:
        del mod_data["groups"][g]
    mod_data.setdefault("groups", {}).update(groups)
    mod_data.setdefault("profiles", {})[name] = {"mods": profile_entries}
    mod_data["active_profile"] = name

    config = json.loads(config_path.read_text()) if config_path.exists() else {"version": "0.0.0"}
    if config.get("version") != "0.0.0":
        sys.exit(f"{config_path}: unsupported config version {config.get('version')!r}")
    config.setdefault("provider_parameters", {})
    if oauth:
        config["provider_parameters"].setdefault("modio", {})["oauth"] = oauth
    if pak:
        config["drg_pak_path"] = pak
    config["ue4ssl_zip_path"] = str(args.ue4ssl_zip)
    config["confirmed_native_dlls"] = sorted(set(config.get("confirmed_native_dlls", [])) | confirmed)
    for key in ("drg_pak_path", "gui_theme", "sorting_config"):
        config.setdefault(key, None)

    enabled = [m for m in mods if m["is_enabled"]]
    print(f"MintCat profile {profile_id} {profile['display_name']!r} -> mint profile {name!r}")
    print(f"  mods: {len(mods)} ({len(enabled)} enabled, {len(mods) - len(enabled)} disabled)")
    for kind in ("Local", "Modio"):
        print(
            f"    {kind}: {sum(m['source_type'] == kind for m in enabled)} enabled, "
            f"{sum(m['source_type'] == kind and not m['is_enabled'] for m in mods)} disabled"
        )
    print(f"  groups: {len(groups)}")
    for g, v in groups.items():
        print(f"    {g}: {len(v['mods'])} mods, {sum(x['enabled'] for x in v['mods'])} enabled")
    print(f"  mod.io token: {'set' if oauth else 'NOT FOUND'}")
    print(f"  drg_pak_path: {pak}")
    print(f"  ue4ssl_zip_path: {args.ue4ssl_zip} ({'exists' if args.ue4ssl_zip.exists() else 'MISSING'})")
    print(f"  native DLLs confirmed: {len(confirmed)} ({', '.join(sorted(native_mods))})")
    for label, items in (
        ("UNMAPPED (skipped)", unmapped),
        ("ENABLED BUT FILE MISSING (integration will fail)", missing_enabled),
        ("version pins ignored (mint uses latest)", pinned),
    ):
        if items:
            print(f"  {label}: {len(items)}")
            for i in items:
                print(f"    {i}")

    if args.dry_run:
        print("dry run: nothing written")
        return
    args.config_dir.mkdir(parents=True, exist_ok=True)
    stamp = time.strftime("%Y%m%d-%H%M%S")
    for path, data in ((mod_data_path, mod_data), (config_path, config)):
        backup(path, stamp)
        tmp = path.with_suffix(".json.tmp")
        tmp.write_text(json.dumps(data, indent=2, ensure_ascii=False))
        tmp.replace(path)
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
