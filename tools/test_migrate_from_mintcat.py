#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Tests for migrate-from-mintcat.py, on a synthetic MintCat database and mint config.

    uv run tools/test_migrate_from_mintcat.py
"""

from __future__ import annotations

import importlib.util
import json
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("migrate-from-mintcat.py")
spec = importlib.util.spec_from_file_location("migrate", SCRIPT)
migrate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(migrate)

PROFILE = "MintCat Default"
KEPT = "/mods/Kept.pak"
ONLY_MINT_A = "/mods/BalancedBhop-0.1.0-alpha.2.pak"
ONLY_MINT_B = "/mods/BalancedBhop-0.1.0-alpha.3.pak"


def make_db(path: Path) -> None:
    con = sqlite3.connect(path)
    con.executescript(
        """
        create table games (id integer primary key, name text, install_path text);
        create table profiles (id integer primary key, name text, display_name text, is_active integer);
        create table profile_folders (id integer primary key, profile_id integer, parent_folder_id integer,
                                      name text, sort_order integer);
        create table mods (mod_id integer primary key, source_type text, url text, display_name text, name_id text);
        create table profile_mods (id integer primary key, profile_id integer, mod_id integer,
                                   parent_folder_id integer, sort_order integer, is_enabled integer,
                                   used_version text);
        create table oauths (id integer primary key, platform text, oauth text);
        insert into games values (1, 'drg', '/game/FSD/Content/Paks/FSD-WindowsNoEditor.pak');
        insert into profiles values (2, 'Default', 'Default', 1);
        insert into profile_folders values (10, 2, null, 'Mine', 0);
        """
    )
    con.execute("insert into mods values (1, 'Local', ?, 'Kept', 'Kept')", (KEPT,))
    con.execute("insert into profile_mods values (1, 2, 1, 10, 0, 1, '')")
    con.commit()
    con.close()


def mod(url: str, enabled: bool = True) -> dict:
    return {"spec": {"url": url}, "required": False, "enabled": enabled}


def mod_data(extra_group_mods: list[dict], extra_profile_mods: list[dict] = ()) -> dict:
    """mint's state after someone added mods directly to mint's copy of the profile."""
    return {
        "version": "0.1.0",
        "active_profile": PROFILE,
        "profiles": {
            PROFILE: {"mods": [{"group_name": "MintCat: Mine", "enabled": True}, *extra_profile_mods]},
            "Other": {"mods": [mod("/mods/OtherProfileOnly.pak")]},
        },
        "groups": {"MintCat: Mine": {"mods": [mod(KEPT), *extra_group_mods]}},
    }


class DroppedEntries(unittest.TestCase):
    new_entries = [{"group_name": "MintCat: Mine", "enabled": True}]
    new_groups = {"MintCat: Mine": {"mods": [mod(KEPT)]}}

    def test_nothing_dropped_when_in_sync(self):
        self.assertEqual(migrate.dropped_entries(mod_data([]), PROFILE, self.new_entries, self.new_groups), [])

    def test_group_and_profile_entries_only_in_mint(self):
        data = mod_data([mod(ONLY_MINT_A)], [mod(ONLY_MINT_B, enabled=False)])
        got = migrate.dropped_entries(data, PROFILE, self.new_entries, self.new_groups)
        self.assertEqual(
            got,
            [("group 'MintCat: Mine'", ONLY_MINT_A), (f"profile {PROFILE!r}", ONLY_MINT_B)],
        )

    def test_stale_mintcat_group_counts_other_profiles_do_not(self):
        data = mod_data([])
        data["groups"]["MintCat: Gone"] = {"mods": [mod(ONLY_MINT_A)]}
        got = migrate.dropped_entries(data, PROFILE, self.new_entries, self.new_groups)
        self.assertEqual(got, [("group 'MintCat: Gone'", ONLY_MINT_A)])

    def test_first_migration(self):
        empty = {"version": "0.1.0", "profiles": {}, "groups": {}}
        self.assertEqual(migrate.dropped_entries(empty, PROFILE, self.new_entries, self.new_groups), [])


class Cli(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.db = self.dir / "mintcat.sqlite"
        make_db(self.db)
        self.cfg = self.dir / "mint"
        self.cfg.mkdir()
        self.mod_data = self.cfg / "mod_data.json"
        self.mod_data.write_text(json.dumps(mod_data([mod(ONLY_MINT_A), mod(ONLY_MINT_B)])))

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, *extra: str) -> subprocess.CompletedProcess:
        cmd = [sys.executable, "-I", str(SCRIPT), "--db", str(self.db), "--config-dir", str(self.cfg),
               "--name", PROFILE, "--ue4ssl-zip", str(self.dir / "UE4SSL.zip"), *extra]
        return subprocess.run(cmd, capture_output=True, text=True)

    def urls(self) -> set[str]:
        data = json.loads(self.mod_data.read_text())
        return migrate.mod_urls(data["profiles"][PROFILE]["mods"], data["groups"])

    def test_refuses_and_writes_nothing(self):
        before = self.mod_data.read_bytes()
        r = self.run_script()
        self.assertEqual(r.returncode, 3, r.stderr)
        self.assertIn("2 entries exist in mintfixed but not in MintCat and would be dropped", r.stdout)
        self.assertIn(ONLY_MINT_A, r.stdout)
        self.assertIn(ONLY_MINT_B, r.stdout)
        self.assertIn("drg-manager add", r.stdout)
        self.assertEqual(self.mod_data.read_bytes(), before)
        self.assertFalse((self.cfg / "config.json").exists())
        self.assertEqual(list(self.cfg.glob("*.bak-*")), [])

    def test_dry_run_lists_without_refusing(self):
        before = self.mod_data.read_bytes()
        r = self.run_script("--dry-run")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(ONLY_MINT_B, r.stdout)
        self.assertIn("dry run: nothing written", r.stdout)
        self.assertEqual(self.mod_data.read_bytes(), before)

    def test_force_drops(self):
        r = self.run_script("--force")
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.urls(), {KEPT})
        self.assertEqual(len(list(self.cfg.glob("mod_data.json.bak-*"))), 1)
        other = json.loads(self.mod_data.read_text())["profiles"]["Other"]
        self.assertEqual(other["mods"][0]["spec"]["url"], "/mods/OtherProfileOnly.pak")

    def test_in_sync_writes_without_force(self):
        self.mod_data.write_text(json.dumps(mod_data([])))
        r = self.run_script()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertNotIn("would be dropped", r.stdout)
        self.assertEqual(self.urls(), {KEPT})


if __name__ == "__main__":
    unittest.main(verbosity=2)
