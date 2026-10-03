//! Finds Elden Ring's `Game` folder: `ELDEN_RING_PATH`, then Steam's
//! registry entries and library folders, then common install paths.
use std::path::{Path, PathBuf};

/// Manual override: the `Game` folder or the `ELDEN RING` folder above it.
pub const ENV_VAR: &str = "ELDEN_RING_PATH";
/// Elden Ring's Steam app id.
pub const STEAM_APP_ID: u32 = 1245620;
const INSTALL_DIR: &str = "ELDEN RING";

/// Whether `dir` looks like Elden Ring's `Game` folder.
pub fn is_game_dir(dir: &Path) -> bool {
    dir.join("eldenring.exe").is_file() || dir.join("Data0.bhd").is_file() || dir.join("regulation.bin").is_file()
}

/// Accepts the `Game` folder itself or the install folder above it.
fn normalise(dir: PathBuf) -> Option<PathBuf> {
    if is_game_dir(&dir) {
        return Some(dir);
    }
    let game = dir.join("Game");
    is_game_dir(&game).then_some(game)
}

/// The Elden Ring `Game` folder, if one can be found.
pub fn find_game_dir() -> Option<PathBuf> {
    candidates().into_iter().find_map(normalise)
}

/// Every place looked at, in order (for diagnostics).
pub fn candidates() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = std::env::var_os(ENV_VAR) {
        out.push(PathBuf::from(dir));
    }
    out.extend(uninstall_registry_location());
    for root in steam_roots() {
        for library in library_folders(&root) {
            out.push(library.join("steamapps").join("common").join(INSTALL_DIR));
        }
    }
    for path in fallback_paths() {
        out.push(PathBuf::from(path));
    }
    out.dedup();
    out
}

/// The library folders listed in a Steam root's `libraryfolders.vdf`,
/// always including the root itself.
pub fn library_folders(steam_root: &Path) -> Vec<PathBuf> {
    let mut out = vec![steam_root.to_path_buf()];
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    if let Ok(text) = std::fs::read_to_string(vdf) {
        out.extend(parse_library_paths(&text));
    }
    out.dedup();
    out
}

/// `"path"  "D:\\SteamLibrary"` lines of a `libraryfolders.vdf`.
fn parse_library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|line| {
            let mut quoted = line.split('"').skip(1).step_by(2);
            (quoted.next()? == "path").then(|| quoted.next().map(|p| PathBuf::from(p.replace("\\\\", "\\"))))
                .flatten()
        })
        .collect()
}

fn fallback_paths() -> Vec<&'static str> {
    if cfg!(windows) {
        vec![
            r"C:\Program Files (x86)\Steam\steamapps\common\ELDEN RING",
            r"C:\Program Files\Steam\steamapps\common\ELDEN RING",
            r"D:\SteamLibrary\steamapps\common\ELDEN RING",
            r"E:\SteamLibrary\steamapps\common\ELDEN RING",
            r"D:\Steam\steamapps\common\ELDEN RING",
        ]
    } else {
        Vec::new()
    }
}

#[cfg(windows)]
fn steam_roots() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let mut out = Vec::new();
    let read = |hive, key: &str, value: &str| -> Option<PathBuf> {
        let key = RegKey::predef(hive).open_subkey(key).ok()?;
        key.get_value::<String, _>(value).ok().map(PathBuf::from)
    };
    out.extend(read(HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"));
    out.extend(read(HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath"));
    out.extend(read(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"));
    out
}

#[cfg(not(windows))]
fn steam_roots() -> Vec<PathBuf> {
    // Linux (Proton) installs, for running the tools off Windows.
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    vec![home.join(".steam/steam"), home.join(".local/share/Steam")]
}

#[cfg(windows)]
fn uninstall_registry_location() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_LOCAL_MACHINE;
    let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\Steam App {STEAM_APP_ID}");
    let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(key).ok()?;
    key.get_value::<String, _>("InstallLocation").ok().map(PathBuf::from)
}

#[cfg(not(windows))]
fn uninstall_registry_location() -> Option<PathBuf> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_library_paths() {
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"C:\\\\Program Files (x86)\\\\Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
        let paths = parse_library_paths(vdf);
        assert_eq!(
            paths,
            vec![PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]
        );
    }
}
