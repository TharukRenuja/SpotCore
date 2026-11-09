//! One application data root, with one subdirectory per concern.

use std::path::{Path, PathBuf};

/// Resolved data paths. Build one with [`Paths::new`] to move the whole SDK
/// somewhere else, or take [`Paths::default`] for the platform location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    /// Fallible platform defaults; credentials never silently fall back to a temporary directory.
    pub fn discover() -> Result<Self, crate::Error> {
        match std::env::var_os("SPOTCORE_DATA_DIR") {
            Some(path) => Ok(Self::new(absolute(path)?)),
            None => Ok(Self::new(platform_root_for(
                std::env::consts::OS,
                &|key| std::env::var_os(key),
            )?)),
        }
    }

    pub fn platform() -> Result<Self, crate::Error> {
        Ok(Self::new(platform_root_for(
            std::env::consts::OS,
            &|key| std::env::var_os(key),
        )?))
    }

    /// Anchors relative roots before asynchronous work can outlive a working-directory change.
    pub fn resolve(&self) -> Result<Self, crate::Error> {
        self.validate()?;
        std::path::absolute(&self.root)
            .map(Self::new)
            .map_err(|error| {
                crate::Error::Backend(format!("cannot resolve data directory: {error}"))
            })
    }

    pub fn prepare(&self) -> Result<(), std::io::Error> {
        let mut builder = std::fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&self.root)
    }

    pub fn validate(&self) -> Result<(), crate::Error> {
        if self.root.as_os_str().is_empty() {
            return Err(crate::Error::InvalidInput("data directory is empty"));
        }
        Ok(())
    }

    /// Isolates an embedding application's accounts, browser profile, and daemon state.
    pub fn for_app(app: &str) -> Result<Self, crate::Error> {
        if app.is_empty()
            || !app
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(crate::Error::InvalidInput(
                "app ID must contain only letters, digits, hyphens or underscores",
            ));
        }
        Ok(Self::new(Self::discover()?.root.join(app)))
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn accounts(&self) -> PathBuf {
        self.root.join("accounts")
    }

    pub fn native(&self) -> PathBuf {
        self.root.join("native")
    }

    pub fn native_data(&self) -> PathBuf {
        self.native().join("data")
    }

    /// Soloist looks for its login inside `<data>/cache`, so the audio cache has
    /// to live under the daemon's data directory rather than beside it.
    pub fn native_cache(&self) -> PathBuf {
        self.native_data().join("cache")
    }

    pub fn profile(&self) -> PathBuf {
        self.root.join("profile")
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    /// The platform data directory, ignoring `SPOTCORE_DATA_DIR`.
    pub fn platform_root() -> PathBuf {
        platform_root()
    }
}

/// `SPOTCORE_DATA_DIR` wins. Panics when no persistent location is available; use `discover` to handle that error.
impl Default for Paths {
    fn default() -> Self {
        Self::new(default_root())
    }
}

fn default_root() -> PathBuf {
    Paths::discover()
        .expect("No persistent data directory; set SPOTCORE_DATA_DIR")
        .root
}

fn platform_root() -> PathBuf {
    platform_root_for(std::env::consts::OS, &|key| std::env::var_os(key))
        .expect("No persistent data directory; set SPOTCORE_DATA_DIR")
}

fn absolute(value: std::ffi::OsString) -> Result<PathBuf, crate::Error> {
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(crate::Error::InvalidInput(
            "platform data directory must be an absolute, nonempty path",
        ));
    }
    Ok(path)
}

fn platform_root_for(
    os: &str,
    get: &impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, crate::Error> {
    let home = || {
        get(if os == "windows" {
            "USERPROFILE"
        } else {
            "HOME"
        })
        .map(absolute)
        .transpose()
    };
    let base = match os {
        "windows" => get("LOCALAPPDATA")
            .or_else(|| get("APPDATA"))
            .map(absolute)
            .transpose()?,
        "macos" => home()?.map(|home| home.join("Library/Application Support")),
        _ => match get("XDG_DATA_HOME") {
            Some(path) => Some(absolute(path)?),
            None => home()?.map(|home| home.join(".local/share")),
        },
    };
    base.map(|base| base.join("spotcore"))
        .ok_or(crate::Error::InvalidInput(
            "no persistent data directory; set SPOTCORE_DATA_DIR or data_dir",
        ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_paths_do_not_fall_back_to_tmp() {
        let base = std::env::temp_dir().join("paths-test");
        let get = |key: &str| match key {
            "HOME" | "USERPROFILE" => Some(base.join("home").into_os_string()),
            "LOCALAPPDATA" => Some(base.join("local").into_os_string()),
            _ => None,
        };
        assert_eq!(
            platform_root_for("linux", &get).unwrap(),
            base.join("home/.local/share/spotcore")
        );
        assert_eq!(
            platform_root_for("macos", &get).unwrap(),
            base.join("home/Library/Application Support/spotcore")
        );
        assert_eq!(
            platform_root_for("windows", &get).unwrap(),
            base.join("local/spotcore")
        );
        assert!(platform_root_for("linux", &|_| None).is_err());
        assert!(platform_root_for("linux", &|_| Some("".into())).is_err());
        assert!(Paths::new("").validate().is_err());
        assert_eq!(
            Paths::new("relative-sdk").resolve().unwrap().root(),
            std::env::current_dir().unwrap().join("relative-sdk")
        );
        assert!(Paths::for_app("../other").is_err());
    }

    #[test]
    fn subdirectories_live_under_one_root() {
        let paths = Paths::new(std::env::temp_dir().join("spotcore"));
        assert_eq!(paths.root(), std::env::temp_dir().join("spotcore"));
        assert_eq!(paths.accounts(), paths.root().join("accounts"));
        assert_eq!(paths.native(), paths.root().join("native"));
        assert_eq!(paths.native_data(), paths.root().join("native/data"));
        assert_eq!(paths.native_cache(), paths.root().join("native/data/cache"));
        assert_eq!(paths.profile(), paths.root().join("profile"));
        assert_eq!(paths.cache(), paths.root().join("cache"));
    }

    #[test]
    fn explicit_root_ignores_the_environment() {
        let dir = std::env::temp_dir().join("spotcore_paths_test");
        assert_eq!(Paths::new(&dir).root(), dir.as_path());
    }
}
