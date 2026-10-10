//! Locating the BDE configuration file.
//!
//! On Windows the active configuration file is recorded in the registry under
//! `HKEY_LOCAL_MACHINE\SOFTWARE\Borland\Database Engine` in the `CONFIGFILE01`
//! value.  When the registry has no (usable) entry we fall back to the usual
//! installation locations.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

/// Registry subkey that stores the active BDE configuration file.
#[cfg(windows)]
const BDE_REGISTRY_SUBKEY: &str = r"SOFTWARE\Borland\Database Engine";
/// Registry value that stores the path to the active configuration file.
#[cfg(windows)]
const BDE_REGISTRY_VALUE: &str = "CONFIGFILE01";

/// Returns the configuration file path recorded in the registry, if any.
#[cfg(windows)]
pub fn registry_config_path() -> Option<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::{
        HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    };

    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let current_user = RegKey::predef(HKEY_CURRENT_USER);

    for hive in [&local_machine, &current_user] {
        // 32-bit view first: BDE itself is a 32-bit component.
        for flags in [
            KEY_READ | KEY_WOW64_32KEY,
            KEY_READ,
            KEY_READ | KEY_WOW64_64KEY,
        ] {
            let Ok(key) = hive.open_subkey_with_flags(BDE_REGISTRY_SUBKEY, flags) else {
                continue;
            };
            let Ok(value) = key.get_value::<String, _>(BDE_REGISTRY_VALUE) else {
                continue;
            };
            if !value.trim().is_empty() {
                return Some(PathBuf::from(value));
            }
        }
    }
    None
}

/// Returns the configuration file path recorded in the registry, if any.
#[cfg(not(windows))]
pub fn registry_config_path() -> Option<PathBuf> {
    None
}

/// Stores `path` in the registry as the active BDE configuration file.
#[cfg(windows)]
pub fn set_registry_config_path(path: &Path) -> Result<()> {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY, KEY_WRITE};

    let value = path.to_string_lossy().to_string();
    let local_machine = RegKey::predef(HKEY_LOCAL_MACHINE);
    let current_user = RegKey::predef(HKEY_CURRENT_USER);

    let mut written = false;
    let mut last_error = None;

    for (hive, flags) in [
        (&local_machine, KEY_WRITE | KEY_WOW64_32KEY),
        (&current_user, KEY_WRITE | KEY_WOW64_32KEY),
        (&local_machine, KEY_WRITE),
        (&current_user, KEY_WRITE),
    ] {
        match hive.create_subkey_with_flags(BDE_REGISTRY_SUBKEY, flags) {
            Ok((key, _)) => match key.set_value(BDE_REGISTRY_VALUE, &value) {
                Ok(()) => written = true,
                Err(error) => last_error = Some(anyhow::Error::from(error)),
            },
            Err(error) => last_error = Some(anyhow::Error::from(error)),
        }
    }

    if written {
        Ok(())
    } else {
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("could not update the Windows registry")))
    }
}

/// Stores `path` in the registry as the active BDE configuration file.
#[cfg(not(windows))]
pub fn set_registry_config_path(_path: &Path) -> Result<()> {
    bail!(
        "--set-default is only supported on Windows: the default BDE configuration file \
         is located through the Windows registry"
    )
}

/// Whether making a file the default BDE configuration file is supported.
pub const fn set_default_supported() -> bool {
    cfg!(windows)
}

/// The usual installation locations of the BDE configuration file.
pub fn standard_config_paths() -> Vec<PathBuf> {
    #[allow(unused_mut)]
    let mut paths = Vec::new();

    #[cfg(windows)]
    {
        const RELATIVE: [(&str, &str); 6] = [
            (
                "ProgramFiles(x86)",
                r"Common Files\Borland Shared\BDE\IDAPI32.CFG",
            ),
            ("ProgramFiles(x86)", r"Borland\Borland Shared\BDE\IDAPI.CFG"),
            (
                "ProgramFiles",
                r"Common Files\Borland Shared\BDE\IDAPI32.CFG",
            ),
            ("ProgramFiles", r"Borland\Borland Shared\BDE\IDAPI.CFG"),
            ("CommonProgramFiles(x86)", r"Borland Shared\BDE\IDAPI32.CFG"),
            ("CommonProgramFiles", r"Borland Shared\BDE\IDAPI32.CFG"),
        ];
        for (variable, relative) in RELATIVE {
            if let Ok(base) = std::env::var(variable)
                && !base.trim().is_empty()
            {
                paths.push(Path::new(&base).join(relative));
            }
        }
    }

    paths
}

/// The path that should be used when no explicit path was given and the file
/// may or may not exist yet.
pub fn default_config_path() -> Option<PathBuf> {
    registry_config_path().or_else(|| standard_config_paths().into_iter().next())
}

/// Resolves the path of an existing configuration file to read from.
pub fn resolve_input_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }

    if let Some(path) = registry_config_path().filter(|path| path.is_file()) {
        return Ok(path);
    }
    if let Some(path) = standard_config_paths()
        .into_iter()
        .find(|path| path.is_file())
    {
        return Ok(path);
    }
    if let Some(path) = registry_config_path() {
        // Let the caller report a useful "file not found" error.
        return Ok(path);
    }
    bail!(
        "cannot locate the Borland BDE configuration file automatically; \
         specify it with --input <PATH>"
    )
}

/// Resolves the path of the configuration file to write to.
pub fn resolve_output_path(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return Ok(path.to_path_buf());
    }
    if let Some(path) = default_config_path() {
        return Ok(path);
    }
    bail!(
        "cannot determine where to write the Borland BDE configuration file; \
         specify it with --output <PATH>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_input_paths_are_returned_as_is() {
        let explicit = Path::new("some/file.cfg");
        assert_eq!(
            resolve_input_path(Some(explicit)).unwrap(),
            explicit.to_path_buf()
        );
    }

    #[test]
    fn explicit_output_paths_are_returned_as_is() {
        let explicit = Path::new("some/out.cfg");
        assert_eq!(
            resolve_output_path(Some(explicit)).unwrap(),
            explicit.to_path_buf()
        );
    }

    #[test]
    fn relative_explicit_paths_are_returned_as_is() {
        // A bare file name must not be resolved against any default location.
        assert_eq!(
            resolve_output_path(Some(Path::new("out.cfg"))).unwrap(),
            PathBuf::from("out.cfg")
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_default_config_path_is_none() {
        assert_eq!(default_config_path(), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_resolve_input_without_explicit_path_fails() {
        let error = resolve_input_path(None).unwrap_err();
        assert!(format!("{error:#}").contains("--input <PATH>"), "{error:#}");
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_resolve_output_without_explicit_path_fails() {
        let error = resolve_output_path(None).unwrap_err();
        assert!(
            format!("{error:#}").contains("--output <PATH>"),
            "{error:#}"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_registry_config_path_is_none() {
        assert_eq!(registry_config_path(), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_set_registry_config_path_fails() {
        let error = set_registry_config_path(Path::new("config.cfg")).unwrap_err();
        assert!(format!("{error:#}").contains("only supported on Windows"));
    }

    #[test]
    fn set_default_supported_matches_the_platform() {
        assert_eq!(set_default_supported(), cfg!(windows));
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_standard_paths_are_empty() {
        // The standard installation locations only exist on Windows, so on
        // other platforms the search space is empty.
        assert!(standard_config_paths().is_empty());
    }
}
