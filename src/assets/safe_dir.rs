//! Directory-handle-relative file operations for the asset installer.
//! Each traversal opens a single component with NOFOLLOW. Destination publication never replaces.
#[cfg(unix)]
mod imp {
    use rustix::fd::OwnedFd;
    use rustix::fs::{self, Mode, OFlags, RenameFlags};
    use std::fs::File;
    use std::path::{Component, Path};

    fn io(err: rustix::io::Errno) -> std::io::Error {
        err.into()
    }
    const DIR_FLAGS: OFlags = OFlags::RDONLY
        .union(OFlags::DIRECTORY)
        .union(OFlags::NOFOLLOW)
        .union(OFlags::CLOEXEC);
    const MODE: Mode = Mode::RUSR.union(Mode::WUSR).union(Mode::XUSR);

    fn check_name(name: &str) -> std::io::Result<()> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
        {
            Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid file name",
            ))
        } else {
            Ok(())
        }
    }

    pub struct SafeDir {
        fd: OwnedFd,
    }
    impl SafeDir {
        /// Open an existing absolute directory without following any path component.
        pub fn open(path: &Path) -> std::io::Result<Self> {
            if !path.is_absolute() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "absolute directory required",
                ));
            }
            let mut fd = fs::open("/", DIR_FLAGS, Mode::empty()).map_err(io)?;
            for component in path.components() {
                match component {
                    Component::RootDir => {}
                    Component::Normal(name) => {
                        fd = fs::openat(&fd, name, DIR_FLAGS, Mode::empty()).map_err(io)?
                    }
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "invalid directory",
                        ))
                    }
                }
            }
            Ok(Self { fd })
        }
        /// Open or create a chain of validated components beneath this directory.
        pub fn directory(&self, path: &str) -> std::io::Result<Self> {
            let mut fd = fs::openat(&self.fd, ".", DIR_FLAGS, Mode::empty()).map_err(io)?;
            if !path.is_empty() {
                for name in path.split('/') {
                    if name.is_empty() || name == "." || name == ".." {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            "invalid component",
                        ));
                    }
                    match fs::mkdirat(&fd, name, MODE) {
                        Ok(()) => fs::fsync(&fd).map_err(io)?,
                        Err(e) if e == rustix::io::Errno::EXIST => {}
                        Err(e) => return Err(io(e)),
                    }
                    fd = fs::openat(&fd, name, DIR_FLAGS, Mode::empty()).map_err(io)?;
                }
            }
            Ok(Self { fd })
        }
        pub fn create_new(&self, name: &str) -> std::io::Result<File> {
            check_name(name)?;
            let flags =
                OFlags::RDWR | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            Ok(File::from(
                fs::openat(&self.fd, name, flags, Mode::RUSR | Mode::WUSR).map_err(io)?,
            ))
        }
        pub fn open_file(&self, name: &str) -> std::io::Result<File> {
            check_name(name)?;
            let flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
            let file = File::from(fs::openat(&self.fd, name, flags, Mode::empty()).map_err(io)?);
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "not a regular file",
                ));
            }
            Ok(file)
        }
        pub fn read_file(&self, name: &str) -> std::io::Result<File> {
            check_name(name)?;
            let flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
            let file = File::from(fs::openat(&self.fd, name, flags, Mode::empty()).map_err(io)?);
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "not a regular file",
                ));
            }
            Ok(file)
        }
        pub fn lock_file(&self, name: &str) -> std::io::Result<File> {
            check_name(name)?;
            let flags = OFlags::RDWR
                | OFlags::CREATE
                | OFlags::NOFOLLOW
                | OFlags::CLOEXEC
                | OFlags::NONBLOCK;
            let fd = fs::openat(&self.fd, name, flags, Mode::RUSR | Mode::WUSR).map_err(io)?;
            let file = File::from(fd);
            if !file.metadata()?.is_file() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "lock is not a regular file",
                ));
            }
            fs::flock(&file, fs::FlockOperation::NonBlockingLockExclusive).map_err(io)?;
            Ok(file)
        }
        pub fn device(&self) -> std::io::Result<u64> {
            Ok(fs::fstat(&self.fd).map_err(io)?.st_dev as u64)
        }
        pub fn remove_file(&self, name: &str) -> std::io::Result<()> {
            check_name(name)?;
            match fs::unlinkat(&self.fd, name, fs::AtFlags::empty()) {
                Ok(()) => fs::fsync(&self.fd).map_err(io),
                Err(e) if e == rustix::io::Errno::NOENT => Ok(()),
                Err(e) => Err(io(e)),
            }
        }
        pub fn replace(&self, from: &str, to: &str) -> std::io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            fs::renameat(&self.fd, from, &self.fd, to).map_err(io)?;
            fs::fsync(&self.fd).map_err(io)
        }
        pub fn publish(&self, from: &str, to_dir: &Self, to: &str) -> std::io::Result<()> {
            check_name(from)?;
            check_name(to)?;
            fs::renameat_with(&self.fd, from, &to_dir.fd, to, RenameFlags::NOREPLACE)
                .map_err(io)?;
            fs::fsync(&to_dir.fd).map_err(io)?;
            fs::fsync(&self.fd).map_err(io)
        }
    }
}
#[cfg(unix)]
pub use imp::SafeDir;
