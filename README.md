# mint + UE4SS.Lite mods

This is a fork of [mintfixed](https://github.com/Wasserkleber/mintfixed) (itself a patch set on
[trumank/mint](https://github.com/trumank/mint)) that can also install
[UE4SS.Lite](https://github.com/iris-cat-dev/UE4SS.Lite) ("UE4SSL") native DLL and JavaScript mods, the same
way [MintCat](https://github.com/iris-cat-dev/mintcat) does.

What it adds:

- A mod `.zip` no longer has to contain a `.pak`. mint looks for three kinds of content:
  - a `.pak` (the first one, as before) — merged into `mods_P.pak` as usual,
  - a native UE4SSL mod — `dll/main.dll` (or else the first `.dll` in the zip, ignoring
    `dwmapi.dll`/`UE4SSL.dll`) — installed to `FSD/Binaries/Win64/ue4ss/mods/<mod>/main.dll`,
  - a UE4SSL JS mod — the `js/` folder that contains `js/main.js` — copied to
    `ue4ss/mods/<mod>/js/`.

  A zip may contain any combination. `<mod>` is the mod's name in mint, which for a local file is
  the file name including `.zip` (e.g. `ue4ss/mods/AntiLag-0.1.0-alpha.5.zip/`), exactly like
  MintCat, so mod settings/log files already in those folders keep working.
- The UE4SSL loader is **not** bundled (UE4SS.Lite has no license). Set the path to a
  `UE4SSL.zip` in the settings window (cogwheel), either a copy you already have (MintCat keeps one
  in its cache folder, e.g. `~/.cache/com.mint.cat/UE4SSL.zip` on Linux) or press **download**,
  which fetches it from MintCat's release server
  (`https://yuri-oss-sg.oss-ap-southeast-1.aliyuncs.com/update.json`) into mint's data directory
  and checks its size and md5. Profiles without DLL/JS mods don't need it.
- On install mint extracts `UE4SSL.zip` into `FSD/Binaries/Win64/` (`dwmapi.dll`,
  `ue4ss/UE4SSL.dll`, `ue4ss/mods/UE4SSL.JavaScript*`) and records everything it created in
  `ue4ss/mods/.mint-managed.json`. "Uninstall mods" (or installing a profile with no DLL/JS mods)
  removes only those files and folders; other folders in `ue4ss/mods/` are left alone. Mods that
  were removed from the profile are deleted on the next install, but a mod's own files (like its
  `.log`) survive reinstalling the same mod.
- The command line works too: `mint integrate --mods Mod.zip ...` uses the `ue4ssl_zip_path` from
  the config.

**Linux / Steam Deck (Proton):** the loader is a `dwmapi.dll` proxy, which Wine ignores unless told
otherwise. Set the game's Steam launch options to:

```
WINEDLLOVERRIDES="dwmapi=n,b" %command%
```

**Don't mix with MintCat.** If MintCat (or the official mod.io integration) has mods installed, turn
that off first (in MintCat: uninstall/disable its integration so `FSD-WindowsNoEditor_Mods.pak` is
gone), otherwise the game loads two merged mod paks.

Credits: [trumank/mint](https://github.com/trumank/mint) (MIT/Apache-2.0), Wasserkleber's
[mintfixed](https://github.com/Wasserkleber/mintfixed) fixes, MintCat for the install layout this
copies, and [UE4SS.Lite](https://github.com/iris-cat-dev/UE4SS.Lite) for the loader itself.

---

This version of Mint fixes these issues:
- Fixed HTTP 403 errors (mod lookup) by removing the deprecated `visible` filter from mod.io API requests.
- Fixed HTTP 403 errors by replacing direct mod file metadata requests (`GET /mods/{id}/files/{file_id}`) with filtered list queries, restoring mod downloads.
- Limits the mission selector/server browser mod list to 100 entries, preventing hosting/invites from breaking when the mission selector mod list string becomes too large.
- Updated Trumans Repaker to make mods that need oodle compression work again automatically without needing to manually add a DLL (for example: https://mod.io/g/drg/m/missions-hud)

If you'd like to build it yourself from the original mint-repo, take a look at my commits to see the fixes I implemented.

# mint

3rd party mod integration tool for Deep Rock Galactic to download and integrate mods completely
externally of the game. This enables more stable mod usage as well as offline mod usage. Works for
both Steam and Microsoft Store versions.

<img alt="Graphical User Interface" src="https://github.com/trumank/mint/assets/1144160/0305419f-a2af-4349-9d63-12e19d97102f">

Mods are added via URL to a .pak or .zip containing a .pak. Mods can also be pulled from mod.io.
Examples:

 - `C:\Path\To\Local\Mod.zip`
 - `https://example.org/some-online-mod-repository/public-mod.pak`
 - `https://mod.io/g/drg/m/sandbox-utilities`

Mods from mod.io will require an OAuth token which can be obtained from <https://mod.io/me/access>
when prompted.

Most mods work just as if they were loaded via the official integration, but there are still some
behavioural differences. If a mod is crashing or otherwise behaving differently than when using the
official integration, *please* create an
[issue](https://github.com/trumank/mint/issues/new) so it can be addressed.

For more details, please consult our [user guide](https://github.com/trumank/mint/wiki).

## Usage

This section assumes that you are on Windows and is using the steam version of DRG, working with
either local `.pak`s or mod.io mods.

First, download the [latest release](https://github.com/trumank/mint/releases/latest)
compatible with your architecture. For windows, this will be the
`mint-x86_64-pc-windows-msvc.zip`. Extract this to anywhere you'd like to keep the
executable.

Then, we'll need to perform some first-time setup.

### First Time Setup

We need to provide the tool with the path to `FSD-WindowsNoEditor.pak` and a mod.io OAuth token if
you want to use mod.io mods. These can be configured in the settings menu (cogwheel located in the
bottom toolbar).

<img alt="Settings menu" src="https://github.com/trumank/mint/assets/1144160/b009a74c-b13a-4b84-95f9-4c59c6debb62">

#### Locating the DRG `FSD-WindowsNoEditor.pak`

If the tool fails to detect your DRG installation, then you can manually browse to add the path to
`FSD-WindowsNoEditor.pak`.

This file is located under the `FSD` folder inside your DRG installation directory, e.g.

```
E:\SteamLibrary\steamapps\common\Deep Rock Galactic\FSD\FSD-WindowsNoEditor.pak
```

#### Adding a mod.io OAuth Token

Inside the settings menu, there is a modio setting (cogwheel). If you click on that, it will prompt
for an mod.io OAuth token.

To generate a mod.io OAuth token, you'll need to visit <https://mod.io/me/access>. You'll need to
accept the API terms and conditions.

<img alt="mod.io Access page" src="https://github.com/trumank/mint/assets/1144160/2aeb6135-71c2-4c3c-8979-49e84b276bed">

Then, you'll need to add a new client under OAuth Access, call it e.g. "DRG Mod Integration".

For that client, create a new token named e.g. "modio-access" with Read-only scope. Copy the token
into the integration tool's prompt.

### Adding Mods

After these steps, you can now add local mods or mod.io mods.

#### Adding mod.io mods

Copy the URL to the mod into the "Add mods..." field and hit enter.

You can obtain a list of your subscribed mods list using the "Copy Mod URLs"
button via [A Better Modding Menu](https://mod.io/g/drg/m/a-better-modding-menu)
in game:

![Copy Mod URLs](https://github.com/trumank/mint/assets/1144160/375f441f-4762-4549-a241-1b54ed391b2f)

#### Adding a local mod

You can either drag and drop a local `.pak` file on to the tool window, or add the path to the
local `.pak` in the same "Add mods..." field.

### Updating Cache

The versioned mod.io mods are *cached*. If you want to update to the latest version of your mods,
you'll need to press the "Update cache" button.

### Installing/uninstalling mods

Once you are happy with your mod profile, you can install the mods by pressing the "Install mods"
button, and uninstall them with the "Uninstall mods" button. **This must be done while the game is
closed.**

## Using integrated mod support again

If you want to go back to the integrated mod support again, you must uninstall the mods installed by
mint. Then, launch the game normally.
