//! Filesystem sources.
//!
//! Paths are always *relative* to the filesystem's root. [`RealFs`] refuses
//! absolute paths and `..` components so a test cannot read or write outside
//! the directory it was given.

use std::collections::BTreeMap;
use std::io;
use std::path::{Component, Path, PathBuf};

/// A minimal filesystem interface covering what tests usually touch.
pub trait Fs: Send {
    /// Reads a whole file.
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>>;
    /// Creates or replaces a file (parent directories are created).
    fn write(&mut self, path: &Path, contents: &[u8]) -> io::Result<()>;
    /// Whether a file or directory exists.
    fn exists(&mut self, path: &Path) -> bool;
    /// Removes a file.
    fn remove(&mut self, path: &Path) -> io::Result<()>;
    /// Names of the entries directly inside a directory, sorted.
    fn list(&mut self, path: &Path) -> io::Result<Vec<String>>;
}

/// Normalizes a relative path, rejecting absolute paths and `..`.
pub fn confine(path: &Path) -> io::Result<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::Normal(p) => out.push(p),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("path {path:?} escapes the filesystem root ('..' is not allowed)"),
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "path {path:?} is absolute; use a path relative to the filesystem root"
                    ),
                ));
            }
        }
    }
    Ok(out)
}

/// An in-memory filesystem. Directories exist implicitly when a file below
/// them exists.
#[derive(Debug, Clone, Default)]
pub struct MemFs {
    files: BTreeMap<PathBuf, Vec<u8>>,
}

impl MemFs {
    /// An empty filesystem.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file (builder style). Invalid paths are ignored.
    pub fn with_file(mut self, path: impl AsRef<Path>, contents: impl Into<Vec<u8>>) -> Self {
        if let Ok(p) = confine(path.as_ref()) {
            self.files.insert(p, contents.into());
        }
        self
    }
}

fn not_found(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} not found", path.display()),
    )
}

impl Fs for MemFs {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        let p = confine(path)?;
        self.files.get(&p).cloned().ok_or_else(|| not_found(path))
    }

    fn write(&mut self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let p = confine(path)?;
        if p.as_os_str().is_empty() {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "empty path"));
        }
        self.files.insert(p, contents.to_vec());
        Ok(())
    }

    fn exists(&mut self, path: &Path) -> bool {
        match confine(path) {
            Ok(p) if p.as_os_str().is_empty() => true,
            Ok(p) => {
                self.files.contains_key(&p)
                    || self.files.keys().any(|k| k.starts_with(&p) && k != &p)
            }
            Err(_) => false,
        }
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        let p = confine(path)?;
        self.files
            .remove(&p)
            .map(|_| ())
            .ok_or_else(|| not_found(path))
    }

    fn list(&mut self, path: &Path) -> io::Result<Vec<String>> {
        let dir = confine(path)?;
        let mut names: Vec<String> = self
            .files
            .keys()
            .filter_map(|k| k.strip_prefix(&dir).ok())
            .filter_map(|rest| rest.components().next())
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names.dedup();
        if names.is_empty() && !dir.as_os_str().is_empty() {
            return Err(not_found(path));
        }
        Ok(names)
    }
}

/// The real filesystem, confined to a root directory. Symlinks inside the
/// root are followed by the OS and are *not* re-checked; see
/// `docs/UNSUPPORTED.md`.
#[derive(Debug, Clone)]
pub struct RealFs {
    root: PathBuf,
}

impl RealFs {
    /// A filesystem rooted at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        RealFs { root: root.into() }
    }

    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        Ok(self.root.join(confine(path)?))
    }
}

impl Fs for RealFs {
    fn read(&mut self, path: &Path) -> io::Result<Vec<u8>> {
        std::fs::read(self.resolve(path)?)
    }

    fn write(&mut self, path: &Path, contents: &[u8]) -> io::Result<()> {
        let p = self.resolve(path)?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(p, contents)
    }

    fn exists(&mut self, path: &Path) -> bool {
        self.resolve(path).map(|p| p.exists()).unwrap_or(false)
    }

    fn remove(&mut self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(self.resolve(path)?)
    }

    fn list(&mut self, path: &Path) -> io::Result<Vec<String>> {
        let mut names = std::fs::read_dir(self.resolve(path)?)?
            .map(|e| e.map(|e| e.file_name().to_string_lossy().into_owned()))
            .collect::<io::Result<Vec<_>>>()?;
        names.sort();
        Ok(names)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn confine_rejects_escapes() {
        assert!(confine(Path::new("../x")).is_err());
        assert!(confine(Path::new("a/../../x")).is_err());
        assert!(confine(Path::new("/etc/passwd")).is_err());
        assert_eq!(confine(Path::new("./a/b")).unwrap(), PathBuf::from("a/b"));
    }

    #[test]
    fn memfs_basic_operations() {
        let mut fs = MemFs::new()
            .with_file("d/a.txt", "1")
            .with_file("d/sub/b.txt", "2");
        assert_eq!(fs.read(Path::new("d/a.txt")).unwrap(), b"1");
        assert!(fs.exists(Path::new("d")));
        assert!(fs.exists(Path::new("d/sub")));
        assert_eq!(fs.list(Path::new("d")).unwrap(), vec!["a.txt", "sub"]);
        fs.remove(Path::new("d/a.txt")).unwrap();
        assert_eq!(
            fs.read(Path::new("d/a.txt")).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert_eq!(
            fs.read(Path::new("../x")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
    }

    #[test]
    fn realfs_is_confined() {
        let dir = tempfile::tempdir().unwrap();
        let mut fs = RealFs::new(dir.path());
        fs.write(Path::new("a/b.txt"), b"hi").unwrap();
        assert_eq!(fs.read(Path::new("a/b.txt")).unwrap(), b"hi");
        assert_eq!(fs.list(Path::new("a")).unwrap(), vec!["b.txt"]);
        assert_eq!(
            fs.read(Path::new("../outside")).unwrap_err().kind(),
            io::ErrorKind::PermissionDenied
        );
        assert!(!fs.exists(Path::new("/etc")));
    }
}
