//! Handle-relative, no-follow file operations for the game-file engine (docs/assets.md
//! "Destination, checkpoint and publication", #309).
//!
//! Every folder and file the engine touches inside the destination or the state folder is
//! reached one component at a time through a `Dir` handle, and no step follows a link:
//!
//! - **Unix** (Linux, Android, macOS, iOS): `openat`/`mkdirat`/`unlinkat`/`renameat2` relative to
//!   a directory descriptor with `O_NOFOLLOW`, so a symlink swapped in for a component after a
//!   check is still refused when it is opened, and publication is `RENAME_NOREPLACE`
//!   (`RENAME_EXCL` on Apple), with a hard link as the fallback where the file system lacks
//!   it. There is never a replacing rename on the publish path.
//! - **Windows**: there are no handle-relative opens in `std`, so each component is opened with
//!   `FILE_FLAG_OPEN_REPARSE_POINT` and refused if it is a reparse point (symlink, junction,
//!   mount point), and the handle of every folder from the root down is held open without
//!   `FILE_SHARE_DELETE`, so Windows refuses to rename or remove a held folder and replace it
//!   with a junction while a run holds it. Publication is `MoveFileExW` without
//!   `MOVEFILE_REPLACE_EXISTING`. Residual risk: a file *leaf* is opened by path, so a
//!   process that can already write inside the app-private folders could still swap a leaf
//!   between the check and the use; the open itself refuses a reparse point, which closes
//!   the swap-to-link case but not a swap to another regular file.
//!
//! The roots (`dest_dir`, `state_dir`) are what the host chose, so they may themselves sit
//! behind links (`/data/user/0` -> `/data/data` on Android, `/var` on macOS); the engine
//! canonicalizes them once and opens the canonical path. On Linux and Android the folders
//! above a root are crossed with search-only (`O_PATH`) handles and elsewhere in one `open`
//! of the canonical path, because an app sandbox lets the app search but not read-open them.
//!
//! A plain-entry failure (a link or a file where a folder belongs, a link where a file
//! belongs) is `io::ErrorKind::InvalidData`; see [`is_not_plain`].

use std::fs::File;
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

/// What a directory entry is, without following it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    File,
    Dir,
    /// A link (symlink, junction), device, socket or anything else that is not ours to follow.
    Other,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Meta {
    pub kind: Kind,
    pub len: u64,
    /// Hard link count (1 where the platform does not say).
    pub nlink: u64,
}

pub(crate) fn not_plain(what: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("{what} is a link, or not the kind of entry that belongs there"),
    )
}

/// A link or the wrong kind of entry stood where a plain folder or file must be.
pub(crate) fn is_not_plain(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::InvalidData
}

fn check_name(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid file name",
        ))
    } else {
        Ok(())
    }
}

#[cfg(unix)]
mod imp {
    use super::*;
    use rustix::fd::OwnedFd;
    use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, RenameFlags};
    use rustix::io::Errno;
    use std::os::unix::fs::MetadataExt;

    const DIR_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const FILE_FLAGS: OFlags = OFlags::NOFOLLOW
        .union(OFlags::CLOEXEC)
        .union(OFlags::NONBLOCK);

    fn dir_mode() -> Mode {
        Mode::RWXU
    }
    fn file_mode() -> Mode {
        Mode::RUSR | Mode::WUSR | Mode::RGRP | Mode::ROTH
    }
    fn io_err(error: Errno) -> io::Error {
        error.into()
    }
    fn dir_err(error: Errno) -> io::Error {
        if error == Errno::LOOP || error == Errno::NOTDIR {
            not_plain("a path component")
        } else {
            error.into()
        }
    }
    fn file_err(error: Errno) -> io::Error {
        if error == Errno::LOOP || error == Errno::ISDIR || error == Errno::NXIO {
            not_plain("a file")
        } else {
            error.into()
        }
    }

    pub(crate) struct Dir {
        fd: OwnedFd,
    }

    impl Dir {
        /// Open an existing absolute (canonical) folder, refusing a link at every component.
        pub(crate) fn open(path: &Path) -> io::Result<Dir> {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "an absolute folder is required",
                ));
            }
            // Only the last component needs read permission. An app's sandbox usually lets it
            // search (x) but not read (r) the folders above its own (`/data` on Android is
            // 0771 system:system), and a read-open of such an ancestor fails with EACCES. So
            // the ancestors are traversed with a search-only handle where the kernel has one
            // (O_PATH), and elsewhere the canonical path is opened in one call, which needs
            // search permission on each ancestor only. The path is canonical (the engine
            // resolved it once and the roots are the host's own choice), so there is no link
            // to follow above the last component, and O_NOFOLLOW refuses one there.
            let components: Vec<Component> = path.components().collect();
            for component in &components {
                if !matches!(component, Component::RootDir | Component::Normal(_)) {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "a canonical folder is required",
                    ));
                }
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            let fd = {
                let walk = DIR_FLAGS.difference(OFlags::RDONLY).union(OFlags::PATH);
                let mut fd = fs::open("/", walk, Mode::empty()).map_err(io_err)?;
                let names: Vec<&std::ffi::OsStr> = components
                    .iter()
                    .filter_map(|c| match c {
                        Component::Normal(name) => Some(*name),
                        _ => None,
                    })
                    .collect();
                match names.split_last() {
                    None => fs::open("/", DIR_FLAGS, Mode::empty()).map_err(io_err)?,
                    Some((last, ancestors)) => {
                        for name in ancestors {
                            fd = fs::openat(&fd, *name, walk, Mode::empty()).map_err(dir_err)?;
                        }
                        fs::openat(&fd, *last, DIR_FLAGS, Mode::empty()).map_err(dir_err)?
                    }
                }
            };
            #[cfg(not(any(target_os = "linux", target_os = "android")))]
            let fd = fs::open(path, DIR_FLAGS, Mode::empty()).map_err(dir_err)?;
            Ok(Dir { fd })
        }

        pub(crate) fn try_clone(&self) -> io::Result<Dir> {
            Ok(Dir {
                fd: self.fd.try_clone()?,
            })
        }

        /// The folder `name` inside this one, created when `create` and missing (the new folder
        /// and this one are then synced). `NotFound` when missing and not `create`.
        pub(crate) fn sub(&self, name: &str, create: bool) -> io::Result<Dir> {
            check_name(name)?;
            let mut created = false;
            if create {
                match fs::mkdirat(&self.fd, name, dir_mode()) {
                    Ok(()) => created = true,
                    Err(e) if e == Errno::EXIST => {}
                    Err(e) => return Err(io_err(e)),
                }
            }
            let fd = fs::openat(&self.fd, name, DIR_FLAGS, Mode::empty()).map_err(dir_err)?;
            if created {
                fs::fsync(&fd).map_err(io_err)?;
                fs::fsync(&self.fd).map_err(io_err)?;
            }
            Ok(Dir { fd })
        }

        // nlink_t differs per platform (see below); the conversion is a no-op on Linux.
        #[allow(clippy::useless_conversion)]
        pub(crate) fn stat(&self, name: &str) -> io::Result<Option<Meta>> {
            check_name(name)?;
            match fs::statat(&self.fd, name, AtFlags::SYMLINK_NOFOLLOW) {
                Ok(st) => {
                    let kind = match FileType::from_raw_mode(st.st_mode as _) {
                        FileType::RegularFile => Kind::File,
                        FileType::Directory => Kind::Dir,
                        _ => Kind::Other,
                    };
                    Ok(Some(Meta {
                        kind,
                        len: st.st_size as u64,
                        // nlink_t is u16 on macOS, u32 on Android, u64 on Linux x86_64.
                        nlink: u64::from(st.st_nlink),
                    }))
                }
                Err(e) if e == Errno::NOENT => Ok(None),
                Err(e) => Err(io_err(e)),
            }
        }

        fn open_with(&self, name: &str, flags: OFlags) -> io::Result<File> {
            check_name(name)?;
            let fd =
                fs::openat(&self.fd, name, flags | FILE_FLAGS, file_mode()).map_err(file_err)?;
            let file = File::from(fd);
            if !file.metadata()?.is_file() {
                return Err(not_plain("a file"));
            }
            Ok(file)
        }

        /// A new regular file; `AlreadyExists` when the name is taken, by anything.
        pub(crate) fn create_new(&self, name: &str) -> io::Result<File> {
            self.open_with(name, OFlags::RDWR | OFlags::CREATE | OFlags::EXCL)
        }

        /// An existing regular file for reading and writing, created when `create`.
        pub(crate) fn open_rw(&self, name: &str, create: bool) -> io::Result<File> {
            let flags = if create {
                OFlags::RDWR | OFlags::CREATE
            } else {
                OFlags::RDWR
            };
            self.open_with(name, flags)
        }

        pub(crate) fn open_read(&self, name: &str) -> io::Result<File> {
            self.open_with(name, OFlags::RDONLY)
        }

        /// Remove a file (or link) entry; a missing name is `NotFound`.
        pub(crate) fn unlink(&self, name: &str) -> io::Result<()> {
            check_name(name)?;
            fs::unlinkat(&self.fd, name, AtFlags::empty()).map_err(io_err)
        }

        /// Remove an empty folder entry.
        pub(crate) fn remove_dir(&self, name: &str) -> io::Result<()> {
            check_name(name)?;
            fs::unlinkat(&self.fd, name, AtFlags::REMOVEDIR).map_err(io_err)
        }

        /// The UTF-8 names in this folder (others are skipped: they are not ours).
        pub(crate) fn names(&self) -> io::Result<Vec<String>> {
            let mut dir = fs::Dir::read_from(&self.fd).map_err(io_err)?;
            let mut names = Vec::new();
            while let Some(entry) = dir.read() {
                let entry = entry.map_err(io_err)?;
                if let Ok(name) = entry.file_name().to_str() {
                    if name != "." && name != ".." {
                        names.push(name.to_string());
                    }
                }
            }
            Ok(names)
        }

        pub(crate) fn sync(&self) -> io::Result<()> {
            fs::fsync(&self.fd).map_err(io_err)
        }

        /// The device the folder is on.
        // dev_t is u64 on Linux but i32 on macOS, so the cast is needed off Linux.
        #[allow(clippy::unnecessary_cast)]
        pub(crate) fn device(&self) -> io::Result<u64> {
            Ok(fs::fstat(&self.fd).map_err(io_err)?.st_dev as u64)
        }

        /// Rename within this folder, replacing `to` (only for the library's own state files).
        pub(crate) fn replace(&self, from: &str, to: &str) -> io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            fs::renameat(&self.fd, from, &self.fd, to).map_err(io_err)
        }

        /// Move `from` (here) to `to` in `to_dir` without replacing anything: `AlreadyExists`
        /// when the name is taken, and then nothing has moved. Both folders are synced.
        pub(crate) fn publish(&self, from: &str, to_dir: &Dir, to: &str) -> io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            match fs::renameat_with(&self.fd, from, &to_dir.fd, to, RenameFlags::NOREPLACE) {
                Ok(()) => {}
                Err(e) if e == Errno::EXIST => {
                    return Err(io::Error::from(io::ErrorKind::AlreadyExists))
                }
                // The file system or the kernel has no no-replace rename: a hard link fails if
                // the name exists, then the part's own name goes. Never a replacing rename.
                Err(e)
                    if e == Errno::NOSYS
                        || e == Errno::INVAL
                        || e == Errno::NOTSUP
                        || e == Errno::PERM =>
                {
                    match fs::linkat(&self.fd, from, &to_dir.fd, to, AtFlags::empty()) {
                        Ok(()) => {
                            let _ = fs::unlinkat(&self.fd, from, AtFlags::empty());
                        }
                        Err(l) if l == Errno::EXIST => {
                            return Err(io::Error::from(io::ErrorKind::AlreadyExists))
                        }
                        Err(l) => {
                            return Err(io::Error::new(
                                io::ErrorKind::Unsupported,
                                format!(
                                    "this file system can neither rename without replacing ({e}) nor hard-link ({l})"
                                ),
                            ))
                        }
                    }
                }
                Err(e) => return Err(io_err(e)),
            }
            fs::fsync(&to_dir.fd).map_err(io_err)?;
            fs::fsync(&self.fd).map_err(io_err)
        }
    }

    /// Hard link count of an open file.
    pub(crate) fn nlink(file: &File) -> io::Result<u64> {
        Ok(file.metadata()?.nlink())
    }

    pub(crate) fn sync_folder(path: &Path) {
        if let Ok(handle) = File::open(path) {
            let _ = handle.sync_all();
        }
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    use std::os::windows::io::AsRawHandle;
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, MoveFileExW, BY_HANDLE_FILE_INFORMATION,
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
        FILE_SHARE_READ, FILE_SHARE_WRITE, MOVEFILE_WRITE_THROUGH,
    };

    fn is_reparse(attributes: u32) -> bool {
        attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }

    /// Open a folder without following it, holding it so it cannot be renamed or removed
    /// (no `FILE_SHARE_DELETE`) for as long as the handle lives.
    fn hold(path: &Path) -> io::Result<Arc<File>> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(path)?;
        let md = file.metadata()?;
        if !md.is_dir() || is_reparse(md.file_attributes()) {
            return Err(not_plain("a path component"));
        }
        Ok(Arc::new(file))
    }

    fn info(file: &File) -> io::Result<BY_HANDLE_FILE_INFORMATION> {
        // SAFETY: a zeroed out-structure and a handle that stays open for the call.
        unsafe {
            let mut info: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
            if GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut info) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(info)
        }
    }

    pub(crate) struct Dir {
        path: PathBuf,
        /// This folder's handle last, after those of every folder above it.
        held: Vec<Arc<File>>,
    }

    impl Dir {
        pub(crate) fn open(path: &Path) -> io::Result<Dir> {
            if !path.is_absolute() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "an absolute folder is required",
                ));
            }
            let mut acc = PathBuf::new();
            let mut held = Vec::new();
            for component in path.components() {
                acc.push(component.as_os_str());
                if matches!(component, Component::Normal(_)) {
                    held.push(hold(&acc)?);
                }
            }
            if held.is_empty() {
                held.push(hold(&acc)?);
            }
            Ok(Dir { path: acc, held })
        }

        pub(crate) fn try_clone(&self) -> io::Result<Dir> {
            Ok(Dir {
                path: self.path.clone(),
                held: self.held.clone(),
            })
        }

        pub(crate) fn sub(&self, name: &str, create: bool) -> io::Result<Dir> {
            check_name(name)?;
            let path = self.path.join(name);
            if create {
                match std::fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(e) => return Err(e),
                }
            }
            let mut held = self.held.clone();
            held.push(hold(&path)?);
            Ok(Dir { path, held })
        }

        pub(crate) fn stat(&self, name: &str) -> io::Result<Option<Meta>> {
            check_name(name)?;
            match std::fs::symlink_metadata(self.path.join(name)) {
                Ok(md) => {
                    let kind = if md.file_type().is_symlink() || is_reparse(md.file_attributes()) {
                        Kind::Other
                    } else if md.is_dir() {
                        Kind::Dir
                    } else if md.is_file() {
                        Kind::File
                    } else {
                        Kind::Other
                    };
                    Ok(Some(Meta {
                        kind,
                        len: md.len(),
                        nlink: 1,
                    }))
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }

        fn open_with(&self, name: &str, options: &mut std::fs::OpenOptions) -> io::Result<File> {
            check_name(name)?;
            let file = options
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))?;
            let md = file.metadata()?;
            if !md.is_file() || is_reparse(md.file_attributes()) {
                return Err(not_plain("a file"));
            }
            Ok(file)
        }

        pub(crate) fn create_new(&self, name: &str) -> io::Result<File> {
            self.open_with(
                name,
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create_new(true),
            )
        }

        pub(crate) fn open_rw(&self, name: &str, create: bool) -> io::Result<File> {
            self.open_with(
                name,
                std::fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(create),
            )
        }

        pub(crate) fn open_read(&self, name: &str) -> io::Result<File> {
            self.open_with(name, std::fs::OpenOptions::new().read(true))
        }

        pub(crate) fn unlink(&self, name: &str) -> io::Result<()> {
            check_name(name)?;
            std::fs::remove_file(self.path.join(name))
        }

        pub(crate) fn remove_dir(&self, name: &str) -> io::Result<()> {
            check_name(name)?;
            std::fs::remove_dir(self.path.join(name))
        }

        pub(crate) fn names(&self) -> io::Result<Vec<String>> {
            let mut names = Vec::new();
            for entry in std::fs::read_dir(&self.path)? {
                if let Ok(name) = entry?.file_name().into_string() {
                    names.push(name);
                }
            }
            Ok(names)
        }

        /// Windows has no folder `fsync` through `std`; NTFS journals its metadata.
        pub(crate) fn sync(&self) -> io::Result<()> {
            Ok(())
        }

        pub(crate) fn device(&self) -> io::Result<u64> {
            let held = self.held.last().expect("a folder holds its own handle");
            Ok(u64::from(info(held)?.dwVolumeSerialNumber))
        }

        pub(crate) fn replace(&self, from: &str, to: &str) -> io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            std::fs::rename(self.path.join(from), self.path.join(to))
        }

        pub(crate) fn publish(&self, from: &str, to_dir: &Dir, to: &str) -> io::Result<()> {
            use std::os::windows::ffi::OsStrExt;
            check_name(from)?;
            check_name(to)?;
            let wide = |path: PathBuf| -> Vec<u16> {
                path.as_os_str().encode_wide().chain(Some(0)).collect()
            };
            let source = wide(self.path.join(from));
            let target = wide(to_dir.path.join(to));
            // Flag 0 for replacing: the move fails if the target exists. No copy across volumes.
            // SAFETY: both are NUL-terminated wide strings that live through the call.
            let moved =
                unsafe { MoveFileExW(source.as_ptr(), target.as_ptr(), MOVEFILE_WRITE_THROUGH) };
            if moved == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
    }

    pub(crate) fn nlink(file: &File) -> io::Result<u64> {
        Ok(u64::from(info(file)?.nNumberOfLinks))
    }

    pub(crate) fn sync_folder(_path: &Path) {}
}

pub(crate) use imp::{nlink, sync_folder, Dir};

/// Largest small state file read (`session.json`, a checkpoint).
pub(crate) const SMALL_MAX: usize = 64 * 1024;

impl Dir {
    /// A small file's bytes, `None` when it does not exist. A link or non-file is
    /// `is_not_plain`; a file over `max` bytes is `InvalidData` too.
    pub(crate) fn read_small(&self, name: &str, max: usize) -> io::Result<Option<Vec<u8>>> {
        let file = match self.open_read(name) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let mut bytes = Vec::new();
        file.take(max as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > max {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "file too long"));
        }
        Ok(Some(bytes))
    }

    /// Crash-safe replacement of one of the library's own files: write a temporary name, sync
    /// it, rename over the target, sync the folder.
    pub(crate) fn write_atomic(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let temp = format!("{name}.tmp");
        match self.unlink(&temp) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let mut file = self.create_new(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        self.replace(&temp, name)?;
        self.sync()
    }

    /// `unlink` that treats a missing name as done.
    pub(crate) fn remove(&self, name: &str) -> io::Result<()> {
        match self.unlink(name) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        }
    }
}

/// `canonicalize` for a path whose tail may not exist yet: the longest existing prefix is
/// resolved, the rest appended. Used to compare roots before anything is created.
pub(crate) fn resolve_lenient(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let mut missing = Vec::new();
    let mut probe = absolute.as_path();
    loop {
        match std::fs::canonicalize(probe) {
            Ok(mut resolved) => {
                resolved.extend(missing.iter().rev());
                return Ok(resolved);
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                match (probe.parent(), probe.file_name()) {
                    (Some(parent), Some(name)) => {
                        missing.push(name.to_os_string());
                        probe = parent;
                    }
                    _ => return Err(e),
                }
            }
            Err(e) => return Err(e),
        }
    }
}

/// `create_dir_all` that then syncs the parent of every folder it made, so none of them can
/// vanish after the call returns.
pub(crate) fn create_dir_all_synced(path: &Path) -> io::Result<()> {
    let mut made = Vec::new();
    let mut probe = path;
    while !probe.exists() {
        made.push(probe.to_path_buf());
        match probe.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => probe = parent,
            _ => break,
        }
    }
    std::fs::create_dir_all(path)?;
    for folder in made.iter().rev() {
        sync_folder(folder);
        if let Some(parent) = folder.parent() {
            sync_folder(parent);
        }
    }
    Ok(())
}
