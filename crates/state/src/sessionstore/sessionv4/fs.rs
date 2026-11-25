use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

use super::error::V4Error;

#[derive(Debug, Clone)]
pub(crate) struct ConfinedRoot {
    root: PathBuf,
}

impl ConfinedRoot {
    pub(crate) fn new(root: &Path) -> Result<Self, V4Error> {
        let resolved = fs::canonicalize(root).map_err(|err| V4Error::io(root, err))?;
        if !resolved.is_dir() {
            return Err(V4Error::io(root, io::Error::other("not a directory")));
        }
        Ok(Self { root: resolved })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    pub(crate) fn resolve(&self, relative: &Path) -> Result<PathBuf, V4Error> {
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(V4Error::io(
                relative,
                io::Error::other("path is not relative"),
            ));
        }
        let mut current = self.root.clone();
        for part in relative.components() {
            current.push(part.as_os_str());
            let info = fs::symlink_metadata(&current).map_err(|err| V4Error::io(&current, err))?;
            if info.file_type().is_symlink() {
                current = fs::canonicalize(&current).map_err(|err| V4Error::io(&current, err))?;
                if !current.starts_with(&self.root) {
                    return Err(V4Error::io(
                        current,
                        io::Error::other("symlink escapes root"),
                    ));
                }
            }
        }
        Ok(current)
    }

    pub(crate) fn read(&self, relative: &Path) -> Result<Vec<u8>, V4Error> {
        let path = self.resolve(relative)?;
        fs::read(&path).map_err(|err| V4Error::io(path, err))
    }

    pub(crate) fn read_limited(&self, relative: &Path, limit: usize) -> Result<Vec<u8>, V4Error> {
        let path = self.resolve(relative)?;
        let file = File::open(&path).map_err(|err| V4Error::io(&path, err))?;
        let mut bytes = Vec::new();
        file.take(limit as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|err| V4Error::io(&path, err))?;
        Ok(bytes)
    }

    pub(crate) fn open(&self, relative: &Path) -> Result<File, V4Error> {
        let path = self.resolve(relative)?;
        File::open(&path).map_err(|err| V4Error::io(path, err))
    }

    pub(crate) fn metadata(&self, relative: &Path) -> Result<fs::Metadata, V4Error> {
        let path = self.resolve(relative)?;
        fs::metadata(&path).map_err(|err| V4Error::io(path, err))
    }
}

pub(crate) fn child_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && Path::new(name).components().count() == 1
        && Path::new(name)
            .components()
            .next()
            .is_some_and(|part| matches!(part, Component::Normal(_)))
}
