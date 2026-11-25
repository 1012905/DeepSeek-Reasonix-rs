use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read};
#[cfg(windows)]
use std::path::Prefix;
use std::path::{Component, Path, PathBuf};

use cap_fs_ext::{FollowSymlinks, MetadataExt as _, OpenOptionsFollowExt};
use cap_std::ambient_authority;
#[cfg(windows)]
use cap_std::fs::OpenOptionsExt as _;
use cap_std::fs::{Dir, File as CapFile, OpenOptions};

use super::error::V4Error;

#[cfg(windows)]
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
#[cfg(windows)]
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
#[cfg(windows)]
const FILE_SHARE_READ_WRITE: u32 = 0x0000_0003;
#[cfg(windows)]
const FILE_SHARE_DELETE: u32 = 0x0000_0004;
const MAX_SYMLINKS: usize = 40;

enum PathPart {
    ParentDir,
    CurDir,
    Normal(OsString),
}

#[derive(Debug, Clone)]
pub(crate) struct ConfinedRoot {
    root: PathBuf,
    identity: (u64, u64),
}

impl ConfinedRoot {
    pub(crate) fn new(root: &Path) -> Result<Self, V4Error> {
        let resolved = fs::canonicalize(root).map_err(|err| V4Error::io(root, err))?;
        let handle = Self::open_dir(&resolved).map_err(|err| V4Error::io(root, err))?;
        let current = Self::open_dir(&resolved).map_err(|err| V4Error::io(root, err))?;
        let identity = dir_identity(&handle).map_err(|err| V4Error::io(root, err))?;
        let same = identity == dir_identity(&current).map_err(|err| V4Error::io(root, err))?;
        let resolved_again = fs::canonicalize(&resolved).map_err(|err| V4Error::io(root, err))?;
        if !same || resolved_again != resolved {
            return Err(V4Error::io(
                root,
                io::Error::other("root changed while opening"),
            ));
        }
        Ok(Self {
            root: resolved,
            identity,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.root
    }

    fn open_dir(path: &Path) -> io::Result<Dir> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        options
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .share_mode(FILE_SHARE_READ_WRITE | FILE_SHARE_DELETE);
        let file = CapFile::open_ambient_with(path, &options, ambient_authority())?;
        if !file.metadata()?.is_dir() {
            return Err(io::Error::other("not a directory"));
        }
        Ok(Dir::from_std_file(file.into_std()))
    }

    fn dir(&self) -> io::Result<Dir> {
        let current = Self::open_dir(&self.root)?;
        // Identity pins the root without retaining a descriptor between reads.
        if dir_identity(&current)? != self.identity {
            return Err(io::Error::other("root directory was replaced"));
        }
        Ok(current)
    }

    fn validate_relative(relative: &Path) -> Result<(), V4Error> {
        if relative
            .components()
            .any(|part| !matches!(part, Component::Normal(_)))
        {
            return Err(V4Error::io(
                relative,
                io::Error::other("path is not relative"),
            ));
        }
        Ok(())
    }

    pub(crate) fn read(&self, relative: &Path) -> Result<Vec<u8>, V4Error> {
        let mut file = self.open(relative)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|err| V4Error::io(relative, err))?;
        Ok(bytes)
    }

    pub(crate) fn read_limited(&self, relative: &Path, limit: usize) -> Result<Vec<u8>, V4Error> {
        let file = self.open(relative)?;
        let mut bytes = Vec::new();
        file.take((limit as u64).saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|err| V4Error::io(relative, err))?;
        Ok(bytes)
    }

    pub(crate) fn open(&self, relative: &Path) -> Result<File, V4Error> {
        Self::validate_relative(relative)?;
        self.open_confined(relative)
            .map_err(|err| V4Error::io(relative, err))
    }

    pub(crate) fn metadata(&self, relative: &Path) -> Result<cap_fs_ext::Metadata, V4Error> {
        Self::validate_relative(relative)?;
        self.metadata_confined(relative)
            .map_err(|err| V4Error::io(relative, err))
    }

    fn open_confined(&self, relative: &Path) -> io::Result<File> {
        let root = self.dir()?;
        let mut current = root.try_clone()?;
        let mut ancestors: Vec<(OsString, Dir)> = Vec::new();
        let mut pending: Vec<PathPart> = relative
            .components()
            .rev()
            .map(|part| match part {
                Component::Normal(part) => PathPart::Normal(part.to_owned()),
                Component::CurDir => PathPart::CurDir,
                Component::ParentDir => PathPart::ParentDir,
                _ => unreachable!("relative paths are validated before opening"),
            })
            .collect();
        let mut symlinks = 0;
        while let Some(part) = pending.pop() {
            match part {
                PathPart::ParentDir => {
                    let Some((_, parent)) = ancestors.pop() else {
                        return Err(io::Error::other("path escapes root"));
                    };
                    current = parent;
                }
                PathPart::CurDir => {}
                PathPart::Normal(name) => {
                    let (file, metadata) = open_entry_nofollow(&current, Path::new(&name))?;
                    if metadata.file_type().is_symlink() {
                        symlinks += 1;
                        if symlinks > MAX_SYMLINKS {
                            return Err(io::Error::other("too many symbolic links"));
                        }
                        let target = read_link(&current, Path::new(&name))?;
                        let target_parts = if target.is_absolute() {
                            let Some(parts) = absolute_relative(&self.root, &target) else {
                                return Err(io::Error::other("symlink escapes root"));
                            };
                            current = root.try_clone()?;
                            ancestors.clear();
                            parts
                        } else {
                            relative_parts(&target)?
                        };
                        let mut expanded: Vec<PathPart> = target_parts
                            .into_iter()
                            .chain(pending.into_iter().rev())
                            .collect();
                        expanded.reverse();
                        pending = expanded;
                        continue;
                    }
                    if pending.is_empty() {
                        return Ok(file.into_std());
                    }
                    if !metadata.is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::NotADirectory,
                            "path component is not a directory",
                        ));
                    }
                    let parent =
                        std::mem::replace(&mut current, Dir::from_std_file(file.into_std()));
                    ancestors.push((name, parent));
                }
            }
        }
        Ok(current.into_std_file())
    }

    fn metadata_confined(&self, relative: &Path) -> io::Result<cap_fs_ext::Metadata> {
        let root = self.dir()?;
        let mut current = root.try_clone()?;
        let mut ancestors: Vec<(OsString, Dir)> = Vec::new();
        let mut pending: Vec<PathPart> = relative
            .components()
            .rev()
            .map(|part| match part {
                Component::Normal(part) => PathPart::Normal(part.to_owned()),
                Component::CurDir => PathPart::CurDir,
                Component::ParentDir => PathPart::ParentDir,
                _ => unreachable!("relative paths are validated before opening"),
            })
            .collect();
        let mut symlinks = 0;
        while let Some(part) = pending.pop() {
            match part {
                PathPart::ParentDir => {
                    let Some((_, parent)) = ancestors.pop() else {
                        return Err(io::Error::other("path escapes root"));
                    };
                    current = parent;
                }
                PathPart::CurDir => {}
                PathPart::Normal(name) => {
                    let metadata = current.symlink_metadata(Path::new(&name))?;
                    if metadata.file_type().is_symlink() {
                        symlinks += 1;
                        if symlinks > MAX_SYMLINKS {
                            return Err(io::Error::other("too many symbolic links"));
                        }
                        let target = read_link(&current, Path::new(&name))?;
                        let target_parts = if target.is_absolute() {
                            let Some(parts) = absolute_relative(&self.root, &target) else {
                                return Err(io::Error::other("symlink escapes root"));
                            };
                            current = root.try_clone()?;
                            ancestors.clear();
                            parts
                        } else {
                            relative_parts(&target)?
                        };
                        let mut expanded: Vec<PathPart> = target_parts
                            .into_iter()
                            .chain(pending.into_iter().rev())
                            .collect();
                        expanded.reverse();
                        pending = expanded;
                        continue;
                    }
                    if pending.is_empty() {
                        return Ok(metadata);
                    }
                    if !metadata.is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::NotADirectory,
                            "path component is not a directory",
                        ));
                    }
                    let (file, opened) = open_entry_nofollow(&current, Path::new(&name))?;
                    if opened.file_type().is_symlink() || !opened.is_dir() {
                        return Err(io::Error::other("directory changed while opening"));
                    }
                    let parent =
                        std::mem::replace(&mut current, Dir::from_std_file(file.into_std()));
                    ancestors.push((name, parent));
                }
            }
        }
        current.dir_metadata()
    }
}

fn dir_identity(dir: &Dir) -> io::Result<(u64, u64)> {
    let metadata = dir.dir_metadata()?;
    Ok((metadata.dev(), metadata.ino()))
}

fn read_link(dir: &Dir, name: &Path) -> io::Result<PathBuf> {
    let target = dir.read_link_contents(name)?;
    #[cfg(windows)]
    {
        // Windows junction reparse buffers may expose a Global\ namespace prefix.
        if let Ok(rest) = target.strip_prefix("Global")
            && rest.is_absolute()
        {
            return Ok(rest.to_path_buf());
        }
    }
    Ok(target)
}

fn open_entry_nofollow(
    dir: &Dir,
    relative: &Path,
) -> io::Result<(cap_std::fs::File, cap_std::fs::Metadata)> {
    let mut options = OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    #[cfg(windows)]
    options
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .share_mode(FILE_SHARE_READ_WRITE);
    let file = dir.open_with(relative, &options)?;
    let metadata = file.metadata()?;
    Ok((file, metadata))
}

fn relative_parts(path: &Path) -> io::Result<Vec<PathPart>> {
    path.components()
        .map(|part| match part {
            Component::Normal(part) => Ok(PathPart::Normal(part.to_owned())),
            Component::CurDir => Ok(PathPart::CurDir),
            Component::ParentDir => Ok(PathPart::ParentDir),
            _ => Err(io::Error::other("symlink target is not relative")),
        })
        .collect()
}

fn absolute_relative(root: &Path, target: &Path) -> Option<Vec<PathPart>> {
    let mut root_components = root.components();
    let mut target_components = target.components();
    loop {
        match (root_components.next(), target_components.next()) {
            (Some(left), Some(right)) if component_eq(&left, &right) => {}
            (Some(_), _) | (None, Some(Component::Prefix(_) | Component::RootDir)) => {
                return None;
            }
            (None, remaining) => {
                let mut parts = Vec::new();
                for component in remaining.into_iter().chain(target_components) {
                    parts.push(path_part(component)?);
                }
                return Some(parts);
            }
        }
    }
}

fn path_part(component: Component<'_>) -> Option<PathPart> {
    match component {
        Component::Normal(part) => Some(PathPart::Normal(part.to_owned())),
        Component::CurDir => Some(PathPart::CurDir),
        Component::ParentDir => Some(PathPart::ParentDir),
        _ => None,
    }
}

fn component_eq(left: &Component<'_>, right: &Component<'_>) -> bool {
    #[cfg(windows)]
    {
        match (left, right) {
            (Component::Prefix(left), Component::Prefix(right)) => {
                match (left.kind(), right.kind()) {
                    (Prefix::VerbatimDisk(left), Prefix::Disk(right))
                    | (Prefix::Disk(left), Prefix::VerbatimDisk(right)) => {
                        left.eq_ignore_ascii_case(&right)
                    }
                    (
                        Prefix::VerbatimUNC(left_server, left_share),
                        Prefix::UNC(right_server, right_share),
                    )
                    | (
                        Prefix::UNC(left_server, left_share),
                        Prefix::VerbatimUNC(right_server, right_share),
                    ) => {
                        left_server
                            .to_string_lossy()
                            .eq_ignore_ascii_case(&right_server.to_string_lossy())
                            && left_share
                                .to_string_lossy()
                                .eq_ignore_ascii_case(&right_share.to_string_lossy())
                    }
                    _ => left
                        .as_os_str()
                        .to_string_lossy()
                        .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy()),
                }
            }
            _ => left
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&right.as_os_str().to_string_lossy()),
        }
    }
    #[cfg(not(windows))]
    {
        left == right
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
