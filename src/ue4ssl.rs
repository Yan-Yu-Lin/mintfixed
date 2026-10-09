//! Support for UE4SS.Lite ("UE4SSL") mods.
//!
//! A UE4SSL mod is a folder under `<Binaries>/ue4ss/mods/` containing a native `main.dll` and/or a
//! JavaScript entry point at `js/main.js`. The loader itself (`dwmapi.dll` proxy, `ue4ss/UE4SSL.dll`
//! and the JavaScript runtime mods) is not bundled with mint; it is extracted from a user supplied
//! `UE4SSL.zip`.
//!
//! Everything mint writes below the binaries directory is recorded in
//! `ue4ss/mods/.mint-managed.json` so that uninstalling only removes what mint created.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::io::{BufReader, Cursor, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

use fs_err as fs;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::integrate::IntegrationError;
use crate::providers::ReadSeek;

/// File (inside `ue4ss/mods/`) listing everything mint installed.
pub const MANIFEST_FILE_NAME: &str = ".mint-managed.json";

/// Files a UE4SSL.zip must contain to be accepted (lowercase, `/` separated).
const REQUIRED_RUNTIME_FILES: [&str; 2] = ["dwmapi.dll", "ue4ss/ue4ssl.dll"];

/// DLLs that belong to the loader itself and are never treated as a mod's native DLL.
const RESERVED_DLL_NAMES: [&str; 2] = ["dwmapi.dll", "ue4ssl.dll"];

/// The parts of a mod archive mint knows how to install.
pub struct ModContent {
    /// Pak to merge into `mods_P.pak`.
    pub pak: Option<Box<dyn ReadSeek>>,
    /// Native UE4SSL mod, installed as `ue4ss/mods/<mod>/main.dll`.
    pub dll: Option<Vec<u8>>,
    /// Files of the JS mod, keyed by their path relative to the mod's `js/` directory.
    pub js: Vec<(PathBuf, Vec<u8>)>,
}

impl ModContent {
    pub fn needs_ue4ssl(&self) -> bool {
        self.dll.is_some() || !self.js.is_empty()
    }
}

fn generic(msg: impl Into<String>) -> IntegrationError {
    IntegrationError::GenericError { msg: msg.into() }
}

fn lowercase_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

/// `a/b/c` style key for a relative path, used in the manifest.
fn slash_key(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn read_entry<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
    index: usize,
) -> Result<Vec<u8>, IntegrationError> {
    let mut file = archive
        .by_index(index)
        .map_err(|_| generic("failed to extract file in zip archive"))?;
    let mut buf = vec![];
    file.read_to_end(&mut buf)?;
    Ok(buf)
}

/// Splits a mod file into its pak, native DLL and JS parts.
///
/// A file that is not a zip is treated as a bare pak (unchanged mint behaviour). For a zip:
/// - pak: the first `.pak` entry (same rule mint always used),
/// - DLL: `dll/main.dll` if present, otherwise the first `.dll` that is not part of the loader,
/// - JS: every file under the `js/` directory that contains `js/main.js`.
///
/// A zip containing none of the three is an error.
pub fn classify_mod(mut data: Box<dyn ReadSeek>) -> Result<ModContent, IntegrationError> {
    let Ok(mut archive) = zip::ZipArchive::new(&mut data) else {
        data.rewind()?;
        return Ok(ModContent {
            pak: Some(data),
            dll: None,
            js: vec![],
        });
    };

    let mut pak_index = None;
    let mut dll_main_index = None;
    let mut dll_any_index = None;
    let mut js_dir: Option<PathBuf> = None;
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|_| generic("failed to extract file in zip archive"))?;
        if !file.is_file() {
            continue;
        }
        let Some(path) = file.enclosed_name() else {
            continue;
        };
        if pak_index.is_none() && path.extension() == Some(OsStr::new("pak")) {
            pak_index = Some(i);
        }
        let components = lowercase_components(&path);
        let [.., parent, name] = components.as_slice() else {
            // single component: only `main.dll`/`foo.dll` at the root are interesting
            if let [name] = components.as_slice()
                && name.ends_with(".dll")
                && !RESERVED_DLL_NAMES.contains(&name.as_str())
                && dll_any_index.is_none()
            {
                dll_any_index = Some(i);
            }
            continue;
        };
        if name.ends_with(".dll") && !RESERVED_DLL_NAMES.contains(&name.as_str()) {
            if dll_main_index.is_none() && parent == "dll" && name == "main.dll" {
                dll_main_index = Some(i);
            }
            if dll_any_index.is_none() {
                dll_any_index = Some(i);
            }
        }
        if js_dir.is_none() && parent == "js" && name == "main.js" {
            js_dir = path.parent().map(Path::to_path_buf);
        }
    }

    let pak = match pak_index {
        Some(i) => Some(Box::new(Cursor::new(read_entry(&mut archive, i)?)) as Box<dyn ReadSeek>),
        None => None,
    };
    let dll = match dll_main_index.or(dll_any_index) {
        Some(i) => Some(read_entry(&mut archive, i)?),
        None => None,
    };
    let mut js = vec![];
    if let Some(js_dir) = &js_dir {
        for i in 0..archive.len() {
            let relative = {
                let file = archive
                    .by_index(i)
                    .map_err(|_| generic("failed to extract file in zip archive"))?;
                if !file.is_file() {
                    continue;
                }
                match file
                    .enclosed_name()
                    .and_then(|p| p.strip_prefix(js_dir).ok().map(Path::to_path_buf))
                {
                    Some(relative) => relative,
                    None => continue,
                }
            };
            js.push((relative, read_entry(&mut archive, i)?));
        }
    }

    if pak.is_none() && dll.is_none() && js.is_empty() {
        return Err(generic(
            "zip archive does not contain a pak, a UE4SSL native DLL (dll/main.dll) or a UE4SSL JS mod (js/main.js)",
        ));
    }
    Ok(ModContent { pak, dll, js })
}

/// Folder name for a mod under `ue4ss/mods/`.
///
/// MintCat names the folder after the mod's name verbatim (for local/HTTP mods that is the file
/// name including `.zip`), so the same is done here to keep per-mod files in the same place.
/// Characters Windows does not allow are replaced.
pub fn mod_folder_name(name: &str) -> String {
    let mut s: String = name
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            {
                '_'
            } else {
                c
            }
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        s = "mod".to_string();
    }
    let stem = s.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|p| {
            stem.strip_prefix(p)
                .is_some_and(|n| n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0")
        });
    // the loader skips a folder called "shared"; a leading dot would collide with the manifest
    if reserved || s.starts_with('.') || s.eq_ignore_ascii_case("shared") {
        s.insert(0, '_');
    }
    s
}

/// Cheap check (central directory only, nothing is decompressed) whether a mod file has a UE4SSL
/// part, so a missing UE4SSL.zip can be reported before anything is written to the game.
pub fn has_ue4ssl_content(path: &Path) -> Result<bool, IntegrationError> {
    let Ok(archive) = zip::ZipArchive::new(BufReader::new(fs::File::open(path)?)) else {
        return Ok(false);
    };
    Ok(archive.file_names().any(|name| {
        let lower = name.replace('\\', "/").to_ascii_lowercase();
        let file = lower.rsplit('/').next().unwrap_or_default();
        lower.ends_with("js/main.js")
            || (file.ends_with(".dll") && !RESERVED_DLL_NAMES.contains(&file))
    }))
}

/// Checks that `path` is a zip that looks like a UE4SSL runtime package.
pub fn validate_ue4ssl_zip(path: &Path) -> Result<(), IntegrationError> {
    let archive = zip::ZipArchive::new(BufReader::new(fs::File::open(path)?))
        .map_err(|e| generic(format!("{} is not a valid zip: {e}", path.display())))?;
    let names = archive
        .file_names()
        .map(|n| n.replace('\\', "/").to_ascii_lowercase())
        .collect::<BTreeSet<_>>();
    for required in REQUIRED_RUNTIME_FILES {
        if !names.contains(required) {
            return Err(generic(format!(
                "{} does not look like UE4SSL.zip (missing {required})",
                path.display()
            )));
        }
    }
    Ok(())
}

/// What mint installed below the game's binaries directory.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Files extracted from UE4SSL.zip that did not exist before mint wrote them, relative to the
    /// binaries directory (`/` separated).
    #[serde(default)]
    pub runtime_files: BTreeSet<String>,
    /// Folders in `ue4ss/mods/` created by extracting UE4SSL.zip (e.g. `UE4SSL.JavaScript`).
    #[serde(default)]
    pub runtime_mod_folders: BTreeSet<String>,
    /// Whether mint created the `ue4ss/` directory.
    #[serde(default)]
    pub created_ue4ss_dir: bool,
    /// Mod folders in `ue4ss/mods/` installed from the mod profile.
    #[serde(default)]
    pub mods: BTreeSet<String>,
}

fn mods_dir(binaries: &Path) -> PathBuf {
    binaries.join("ue4ss").join("mods")
}

fn manifest_path(binaries: &Path) -> PathBuf {
    mods_dir(binaries).join(MANIFEST_FILE_NAME)
}

pub fn read_manifest(binaries: &Path) -> Result<Option<Manifest>, IntegrationError> {
    match std::fs::read(manifest_path(binaries)) {
        Ok(data) => serde_json::from_slice(&data)
            .map(Some)
            .map_err(|e| generic(format!("failed to parse {MANIFEST_FILE_NAME}: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_manifest(binaries: &Path, manifest: &Manifest) -> Result<(), IntegrationError> {
    let path = manifest_path(binaries);
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, serde_json::to_vec_pretty(manifest).unwrap())?;
    fs::rename(&tmp, &path)?;
    Ok(())
}

fn remove_file_if_exists(path: &Path) -> Result<(), IntegrationError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

fn remove_dir_if_exists(path: &Path) -> Result<(), IntegrationError> {
    match std::fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(generic(format!("failed to remove {}: {e}", path.display()))),
    }
}

/// Writes `data` to `path` via a temporary file and a rename.
fn write_atomic(path: &Path, data: &[u8]) -> Result<(), IntegrationError> {
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut file = fs::File::create(&tmp)?;
        file.write_all(data)?;
        file.flush()?;
    }
    remove_file_if_exists(path)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

/// Extracts UE4SSL.zip into the binaries directory, recording newly created files in `manifest`.
fn install_runtime(
    binaries: &Path,
    ue4ssl_zip: &Path,
    manifest: &mut Manifest,
) -> Result<(), IntegrationError> {
    validate_ue4ssl_zip(ue4ssl_zip)?;
    let mut archive = zip::ZipArchive::new(BufReader::new(fs::File::open(ue4ssl_zip)?))
        .map_err(|e| generic(format!("failed to open UE4SSL.zip: {e}")))?;
    for i in 0..archive.len() {
        let (relative, is_file) = {
            let file = archive
                .by_index(i)
                .map_err(|_| generic("failed to extract file in UE4SSL.zip"))?;
            match file.enclosed_name() {
                Some(p) => (p, file.is_file()),
                None => {
                    warn!("skipping unsafe path in UE4SSL.zip: {}", file.name());
                    continue;
                }
            }
        };
        if !is_file {
            continue;
        }
        let target = binaries.join(&relative);
        let key = slash_key(&relative);
        let components = lowercase_components(&relative);
        if let [ue4ss, mods, folder, _, ..] = components.as_slice()
            && ue4ss == "ue4ss"
            && mods == "mods"
        {
            // runtime mod folder, e.g. ue4ss/mods/UE4SSL.JavaScript/main.dll
            let folder_name = relative
                .components()
                .nth(2)
                .unwrap()
                .as_os_str()
                .to_string_lossy()
                .into_owned();
            debug_assert_eq!(folder_name.to_ascii_lowercase(), *folder);
            if !manifest.runtime_mod_folders.contains(&folder_name)
                && !mods_dir(binaries).join(&folder_name).exists()
            {
                manifest.runtime_mod_folders.insert(folder_name);
            }
        } else if !target.exists() {
            manifest.runtime_files.insert(key);
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = read_entry(&mut archive, i)?;
        write_atomic(&target, &data)?;
    }
    Ok(())
}

/// A mod's UE4SSL parts, ready to be written to `ue4ss/mods/<folder>/`.
pub struct Ue4sslMod {
    pub folder: String,
    pub dll: Option<Vec<u8>>,
    pub js: Vec<(PathBuf, Vec<u8>)>,
}

/// Installs the UE4SSL runtime and the given mods, removing mint-managed mods that are no longer
/// in the list. With no mods, everything mint installed is removed instead.
pub fn install(
    binaries: &Path,
    ue4ssl_zip: Option<&Path>,
    mods: &[Ue4sslMod],
) -> Result<(), IntegrationError> {
    if mods.is_empty() {
        return uninstall(binaries);
    }
    let ue4ssl_zip = ue4ssl_zip.ok_or_else(|| {
        generic("this profile has UE4SSL mods but no UE4SSL.zip is configured (see settings)")
    })?;

    let mut seen = BTreeSet::new();
    for m in mods {
        if !seen.insert(m.folder.to_ascii_lowercase()) {
            return Err(generic(format!(
                "two UE4SSL mods would be installed to the same folder ue4ss/mods/{}",
                m.folder
            )));
        }
    }

    let mut manifest = read_manifest(binaries)?.unwrap_or_default();
    if !binaries.join("ue4ss").exists() {
        manifest.created_ue4ss_dir = true;
    }
    let mods_dir = mods_dir(binaries);
    fs::create_dir_all(&mods_dir)?;
    // record ownership before touching anything else, so a failure below can still be undone
    write_manifest(binaries, &manifest)?;

    install_runtime(binaries, ue4ssl_zip, &mut manifest)?;
    write_manifest(binaries, &manifest)?;

    let wanted = mods.iter().map(|m| m.folder.clone()).collect::<BTreeSet<_>>();
    for stale in manifest.mods.difference(&wanted).cloned().collect::<Vec<_>>() {
        info!("removing UE4SSL mod no longer in profile: {stale}");
        remove_dir_if_exists(&mods_dir.join(&stale))?;
        manifest.mods.remove(&stale);
    }

    for m in mods {
        let dir = mods_dir.join(&m.folder);
        manifest.mods.insert(m.folder.clone());
        write_manifest(binaries, &manifest)?;
        fs::create_dir_all(&dir)?;
        // replace the code but keep anything else in the folder (e.g. the mod's log)
        remove_file_if_exists(&dir.join("main.dll.tmp"))?;
        remove_dir_if_exists(&dir.join("js"))?;
        match &m.dll {
            Some(dll) => write_atomic(&dir.join("main.dll"), dll)?,
            None => remove_file_if_exists(&dir.join("main.dll"))?,
        }
        for (relative, data) in &m.js {
            let target = dir.join("js").join(relative);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&target, data)?;
        }
        info!("installed UE4SSL mod to {}", dir.display());
    }
    write_manifest(binaries, &manifest)?;
    Ok(())
}

/// Removes everything listed in the mint manifest. Files and folders mint did not create are left
/// alone. Does nothing if there is no manifest.
pub fn uninstall(binaries: &Path) -> Result<(), IntegrationError> {
    let Some(manifest) = read_manifest(binaries)? else {
        return Ok(());
    };
    let mods_dir = mods_dir(binaries);
    for folder in manifest.mods.iter().chain(&manifest.runtime_mod_folders) {
        if folder.is_empty() || folder.contains(['/', '\\']) || folder.starts_with('.') {
            warn!("ignoring suspicious manifest entry {folder:?}");
            continue;
        }
        remove_dir_if_exists(&mods_dir.join(folder))?;
    }
    for file in &manifest.runtime_files {
        let relative = Path::new(file);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            warn!("ignoring suspicious manifest entry {file:?}");
            continue;
        }
        remove_file_if_exists(&binaries.join(relative))?;
    }
    remove_file_if_exists(&manifest_path(binaries))?;

    // tidy up directories mint created if nothing foreign is left in them
    let _ = std::fs::remove_dir(&mods_dir);
    if manifest.created_ue4ss_dir {
        let ue4ss = binaries.join("ue4ss");
        let only_logs = std::fs::read_dir(&ue4ss)
            .map(|entries| {
                entries.flatten().all(|e| {
                    e.file_type().is_ok_and(|t| t.is_file())
                        && e.path().extension() == Some(OsStr::new("log"))
                })
            })
            .unwrap_or(false);
        if only_logs {
            remove_dir_if_exists(&ue4ss)?;
        }
    }
    info!("removed UE4SSL files installed by mint");
    Ok(())
}

/// Static release manifest MintCat uses for its own downloads (international mirror).
pub const MINTCAT_MANIFEST_URL: &str =
    "https://yuri-oss-sg.oss-ap-southeast-1.aliyuncs.com/update.json";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ManifestItem {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    platform: Option<String>,
    #[serde(default)]
    latest_version: Option<String>,
    #[serde(default)]
    file_size: Option<u64>,
    #[serde(default)]
    md5: Option<String>,
    #[serde(default)]
    download_url: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    path: Option<String>,
}

/// Picks the stable windows `ue4ssl` entry and resolves its download URL relative to the manifest.
fn resolve_ue4ssl_asset(
    manifest_url: &str,
    items: &[ManifestItem],
) -> Result<(String, Option<u64>, Option<String>, String), IntegrationError> {
    let eq = |v: &Option<String>, want: &str| {
        v.as_deref()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case(want)
    };
    let item = items
        .iter()
        .find(|i| {
            eq(&i.name, "ue4ssl")
                && (i.channel.is_none() || eq(&i.channel, "stable"))
                && (i.platform.is_none() || eq(&i.platform, "windows"))
        })
        .ok_or_else(|| generic("MintCat's update manifest has no stable ue4ssl entry"))?;
    let relative = item
        .download_url
        .as_ref()
        .or(item.url.as_ref())
        .or(item.path.as_ref())
        .cloned()
        .unwrap_or_else(|| "UE4SSL.zip".to_string());
    let url = url::Url::parse(manifest_url)
        .and_then(|base| base.join(relative.trim_start_matches('/')))
        .map_err(|e| generic(format!("bad UE4SSL download url {relative:?}: {e}")))?;
    Ok((
        url.to_string(),
        item.file_size,
        item.md5.clone(),
        item.latest_version.clone().unwrap_or_default(),
    ))
}

/// Downloads UE4SSL.zip from MintCat's release server to `dest`, checking its size, md5 and
/// contents. Returns the version string from the manifest.
pub async fn download_ue4ssl(dest: &Path) -> Result<String, IntegrationError> {
    let client = reqwest::Client::new();
    let fetch_err = |e: reqwest::Error| generic(format!("UE4SSL download failed: {e}"));
    let items: Vec<ManifestItem> = client
        .get(MINTCAT_MANIFEST_URL)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(fetch_err)?
        .json()
        .await
        .map_err(fetch_err)?;
    let (url, size, md5, version) = resolve_ue4ssl_asset(MINTCAT_MANIFEST_URL, &items)?;
    info!("downloading UE4SSL {version} from {url}");
    let data = client
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(fetch_err)?
        .bytes()
        .await
        .map_err(fetch_err)?;
    if let Some(size) = size
        && size != data.len() as u64
    {
        return Err(generic(format!(
            "UE4SSL download is {} bytes, expected {size}",
            data.len()
        )));
    }
    if let Some(md5) = md5 {
        let actual = hex::encode(<md5::Md5 as md5::Digest>::digest(&data));
        if !actual.eq_ignore_ascii_case(md5.trim()) {
            return Err(generic(format!(
                "UE4SSL download md5 mismatch: got {actual}, expected {md5}"
            )));
        }
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = dest.with_extension("zip.part");
    fs::write(&tmp, &data)?;
    if let Err(e) = validate_ue4ssl_zip(&tmp) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, dest)?;
    Ok(version)
}
