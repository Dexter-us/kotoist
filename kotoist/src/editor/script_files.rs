use std::fs;
use std::path::{Path, PathBuf};

const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;

#[cfg(windows)]
pub(crate) fn directory() -> Result<PathBuf, String> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use winapi::shared::minwindef::HMODULE;
    use winapi::um::libloaderapi::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    let mut module: HMODULE = std::ptr::null_mut();
    let address = directory as *const () as *const u16;
    let flags =
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
    if unsafe { GetModuleHandleExW(flags, address, &mut module) } == 0 {
        return Err(format!(
            "Could not locate the Kotoist plugin module: {}",
            std::io::Error::last_os_error()
        ));
    }

    let mut buffer = vec![0_u16; 32_768];
    let length = unsafe { GetModuleFileNameW(module, buffer.as_mut_ptr(), buffer.len() as u32) };
    if length == 0 || length as usize >= buffer.len() {
        return Err(format!(
            "Could not read the Kotoist plugin path: {}",
            std::io::Error::last_os_error()
        ));
    }

    let module_file = PathBuf::from(OsString::from_wide(&buffer[..length as usize]));
    let bundle = module_file.ancestors().find(|ancestor| {
        ancestor
            .extension()
            .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("vst3"))
            && ancestor.is_dir()
    });
    let plugin_root = bundle
        .map(PathBuf::from)
        .or_else(|| module_file.parent().map(Path::to_path_buf))
        .ok_or_else(|| "Could not find the Kotoist plugin folder.".to_owned())?;

    Ok(plugin_root.join("Dexter U.S. Kotoist").join("Scripts"))
}

#[cfg(not(windows))]
pub(crate) fn directory() -> Result<PathBuf, String> {
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

    Ok(base.join("Dexter U.S.").join("Kotoist").join("Scripts"))
}

pub(crate) fn list() -> Result<Vec<PathBuf>, String> {
    let directory = directory()?;
    fs::create_dir_all(&directory)
        .map_err(|error| {
            format!(
                "Could not create the script folder inside the plugin: {error}. If Kotoist is installed under Program Files, move the .vst3 bundle to a user-writable VST3 folder."
            )
        })?;

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
        .map_err(|error| {
            format!(
                "Could not create the script folder inside the plugin: {error}. If Kotoist is installed under Program Files, move the .vst3 bundle to a user-writable VST3 folder."
            )
        })?;

    let path = directory.join(format!("{name}.koto"));
    fs::write(&path, contents).map_err(|error| {
        format!(
            "Could not save the script inside the plugin: {error}. If Kotoist is installed under Program Files, move the .vst3 bundle to a user-writable VST3 folder."
        )
    })?;
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

    let device_name = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ["COM", "LPT"].iter().any(|prefix| {
            device_name.strip_prefix(prefix).is_some_and(|suffix| {
                matches!(suffix, "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9")
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
