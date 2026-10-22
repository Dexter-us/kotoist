use std::fs;
use std::path::{Path, PathBuf};

const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

pub(crate) fn directory() -> Result<PathBuf, String> {
    #[cfg(windows)]
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "Windows did not provide the LOCALAPPDATA folder.".to_owned())?;

    #[cfg(target_os = "macos")]
    let base = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| "Could not find the user home folder.".to_owned())?
        .join("Library")
        .join("Application Support");

    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local").join("share"))
        })
        .ok_or_else(|| "Could not find the local application-data folder.".to_owned())?;

    #[cfg(not(any(windows, unix)))]
    let base = std::env::current_dir()
        .map_err(|error| format!("Could not find a local data folder: {error}"))?;

    Ok(base
        .join("Dexter U.S.")
        .join("Kotoist")
        .join("Scripts"))
}

pub(crate) fn list() -> Result<Vec<PathBuf>, String> {
    let directory = directory()?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the Kotoist script folder: {error}"))?;

    let mut files = fs::read_dir(&directory)
        .map_err(|error| format!("Could not read the Kotoist script folder: {error}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.extension().is_some_and(|extension| {
                    extension.to_string_lossy().eq_ignore_ascii_case("koto")
                })
        })
        .collect::<Vec<_>>();

    files.sort_by_key(|path| {
        path.file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase()
    });
    Ok(files)
}

pub(crate) fn load(path: &Path) -> Result<String, String> {
    let directory = directory()?;
    if path.parent() != Some(directory.as_path())
        || !path
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("koto"))
    {
        return Err("Choose a .koto file from the Kotoist Scripts folder.".to_owned());
    }

    let metadata =
        fs::metadata(path).map_err(|error| format!("Could not open the script: {error}"))?;
    if metadata.len() > MAX_SCRIPT_BYTES {
        return Err("That script is larger than the 1 MB limit.".to_owned());
    }
    fs::read_to_string(path).map_err(|error| format!("Could not read the script: {error}"))
}

pub(crate) fn save(name: &str, contents: &str) -> Result<PathBuf, String> {
    if contents.len() as u64 > MAX_SCRIPT_BYTES {
        return Err("That script is larger than the 1 MB limit.".to_owned());
    }

    let name = normalize_name(name)?;
    let directory = directory()?;
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create the Kotoist script folder: {error}"))?;

    let path = directory.join(format!("{name}.koto"));
    fs::write(&path, contents).map_err(|error| format!("Could not save the script: {error}"))?;
    Ok(path)
}

fn normalize_name(name: &str) -> Result<String, String> {
    let mut name = name.trim();
    if let Some((stem, extension)) = name.rsplit_once('.') {
        if extension.eq_ignore_ascii_case("koto") {
            name = stem;
        }
    }

    if name.is_empty()
        || name == "."
        || name == ".."
        || name.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err("Enter a file name without Windows path or reserved characters.".to_owned());
    }

    let device_name = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device_name.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(
                    suffix,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9"
                )
            })
        });
    if reserved {
        return Err("That name is reserved by Windows; choose another name.".to_owned());
    }

    Ok(name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::normalize_name;

    #[test]
    fn adds_the_koto_extension_once() {
        assert_eq!(normalize_name("  Arpeggio  ").unwrap(), "Arpeggio");
        assert_eq!(normalize_name("Arpeggio.koto").unwrap(), "Arpeggio");
    }

    #[test]
    fn rejects_paths_and_windows_device_names() {
        assert!(normalize_name("../outside").is_err());
        assert!(normalize_name("C:\\outside").is_err());
        assert!(normalize_name("CON").is_err());
    }
}
