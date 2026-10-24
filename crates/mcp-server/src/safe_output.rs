//! Exclusive output creation with source-image isolation.
use anyhow::{bail, Context, Result};
use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

pub fn validate_filename(filename: &str) -> Result<()> {
    if filename.is_empty()
        || filename.len() > 240
        || filename.ends_with(['.', ' '])
        || filename
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        || filename == "."
        || filename == ".."
    {
        bail!("Unsafe output filename");
    }
    let stem = filename
        .split('.')
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit())
    {
        bail!("Reserved device filename");
    }
    Ok(())
}

pub fn output_path(source: &str, directory: &Path, filename: &str) -> Result<PathBuf> {
    validate_filename(filename)?;
    if source.starts_with(r"\\.\") || source.starts_with("/dev/") {
        bail!("NotImplemented: exporting from a raw device requires verified destination-device isolation");
    }
    let source = fs::canonicalize(source).context("Cannot canonicalize source; refusing output")?;
    if !source.is_file() {
        bail!("NotImplemented: exporting from a device requires verified destination-device isolation");
    }
    if Path::new(filename).components().count() != 1 || filename == "." || filename == ".." {
        bail!("Invalid output filename");
    }
    // Resolve existing ancestors before creating directories, including symlinks.
    let mut ancestor = directory.to_path_buf();
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        suffix.push(
            ancestor
                .file_name()
                .context("Invalid output directory")?
                .to_os_string(),
        );
        if !ancestor.pop() {
            bail!("Cannot resolve output directory");
        }
        if ancestor.as_os_str().is_empty() {
            ancestor = std::env::current_dir()?;
        }
    }
    let mut resolved = fs::canonicalize(&ancestor)?;
    for part in suffix.iter().rev() {
        resolved.push(part);
    }
    if resolved.starts_with(&source) {
        bail!("Output resides inside source path");
    }
    fs::create_dir_all(&resolved)?;
    let parent = fs::canonicalize(&resolved)?;
    let target = parent.join(filename);
    let normalized = if target.exists() {
        fs::canonicalize(&target)?
    } else {
        target.clone()
    };
    if normalized == source || normalized.starts_with(&source) {
        bail!("Output matches or resides inside source image path");
    }
    Ok(target)
}

/// Holds the canonical output directory open for the entire write/commit.
pub struct DirectoryGuard {
    path: PathBuf,
    directory: File,
    #[cfg(windows)]
    _ancestors: Vec<File>,
}
impl DirectoryGuard {
    fn open(path: &Path) -> Result<Self> {
        let path = fs::canonicalize(path)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::*;
            let mut handles = Vec::new();
            for ancestor in path.ancestors() {
                let handle = OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
                    .open(ancestor)?;
                let info = file_identity(&handle)?;
                anyhow::ensure!(
                    info.3 & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                    "Reparse point in pinned output path"
                );
                anyhow::ensure!(
                    same_path(&final_path(&handle)?, ancestor),
                    "Output ancestor changed during opening"
                );
                handles.push(handle);
            }
            let directory = handles
                .first()
                .context("Missing directory handle")?
                .try_clone()?;
            Ok(Self {
                path,
                directory,
                _ancestors: handles,
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let directory = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&path)?;
            anyhow::ensure!(
                same_file(&directory, &File::open(&path)?)?,
                "Output directory changed during opening"
            );
            Ok(Self { path, directory })
        }
        #[cfg(not(any(windows, unix)))]
        {
            anyhow::bail!("NotImplemented: secure directory handles on this platform")
        }
    }
    fn create(&self, source: &str, name: &str) -> Result<File> {
        validate_filename(name)?;
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::{Foundation::GENERIC_WRITE, Storage::FileSystem::*};
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .access_mode(GENERIC_WRITE | DELETE)
                .share_mode(FILE_SHARE_READ)
                .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
                .open(self.path.join(name))?
        };
        #[cfg(unix)]
        let file = {
            use std::os::fd::{AsRawFd, FromRawFd};
            use std::os::unix::ffi::OsStrExt;
            let name = std::ffi::CString::new(std::ffi::OsStr::new(name).as_bytes())?;
            let fd = unsafe {
                libc::openat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_WRONLY
                        | libc::O_CREAT
                        | libc::O_EXCL
                        | libc::O_NOFOLLOW
                        | libc::O_CLOEXEC,
                    0o600,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            unsafe { File::from_raw_fd(fd) }
        };
        let source = File::open(source)?;
        anyhow::ensure!(
            !same_file(&source, &file)?,
            "Output aliases source image handle"
        );
        self.verify(&file, name)?;
        Ok(file)
    }
    fn verify(&self, file: &File, name: &str) -> Result<()> {
        #[cfg(windows)]
        {
            use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;
            anyhow::ensure!(
                file_identity(file)?.3 & FILE_ATTRIBUTE_REPARSE_POINT == 0,
                "Output is a reparse point"
            );
            anyhow::ensure!(
                same_path(&final_path(&self.directory)?, &self.path),
                "Pinned output directory was relocated"
            );
            let actual = final_path(file)?;
            let expected = self.path.join(name);
            anyhow::ensure!(
                same_path(&actual, &expected),
                "Output handle escaped canonical directory: actual={}, expected={}",
                actual.display(),
                expected.display()
            );
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            use std::os::unix::ffi::OsStrExt;
            use std::os::unix::fs::MetadataExt;
            let current = File::open(&self.path)?;
            anyhow::ensure!(
                same_file(&self.directory, &current)?,
                "Output directory path was replaced"
            );
            let name = std::ffi::CString::new(std::ffi::OsStr::new(name).as_bytes())?;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::zeroed();
            if unsafe {
                libc::fstatat(
                    self.directory.as_raw_fd(),
                    name.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let stat = unsafe { stat.assume_init() };
            let meta = file.metadata()?;
            // dev_t/ino_t widths vary across Unix targets.
            #[allow(clippy::unnecessary_cast)]
            let matches = stat.st_dev as u64 == meta.dev()
                && stat.st_ino as u64 == meta.ino()
                && stat.st_mode & libc::S_IFMT == libc::S_IFREG;
            anyhow::ensure!(matches, "Output handle/path identity mismatch");
        }
        Ok(())
    }
    fn rename(&self, file: &File, from: &mut String, to: &str) -> Result<()> {
        self.verify(file, from)?;
        validate_filename(to)?;
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::*;
            let name: Vec<u16> = self
                .path
                .join(to)
                .as_os_str()
                .to_string_lossy()
                .encode_utf16()
                .collect();
            let bytes = std::mem::offset_of!(FILE_RENAME_INFO, FileName) + (name.len() + 1) * 2;
            let mut buffer = vec![0usize; bytes.div_ceil(std::mem::size_of::<usize>())];
            let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
            unsafe {
                (*info).Anonymous.ReplaceIfExists = 0;
                (*info).RootDirectory = std::ptr::null_mut();
                (*info).FileNameLength = (name.len() * 2) as u32;
                std::ptr::copy_nonoverlapping(
                    name.as_ptr(),
                    std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
                    name.len(),
                );
                if SetFileInformationByHandle(
                    file.as_raw_handle() as _,
                    FileRenameInfo,
                    info.cast(),
                    bytes as u32,
                ) == 0
                {
                    return Err(std::io::Error::last_os_error().into());
                }
            }
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let old = std::ffi::CString::new(from.as_str())?;
            let new = std::ffi::CString::new(to)?;
            let fd = self.directory.as_raw_fd();
            // linkat+unlinkat never replaces an existing destination. Both names
            // refer to the same already-verified inode until the old link is removed.
            if unsafe { libc::linkat(fd, old.as_ptr(), fd, new.as_ptr(), 0) } != 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe { libc::unlinkat(fd, old.as_ptr(), 0) } != 0 {
                unsafe { libc::unlinkat(fd, new.as_ptr(), 0) };
                return Err(std::io::Error::last_os_error().into());
            }
        }
        // Track the kernel mutation before fallible verification/fsync. Recovery
        // must use the new name even when either of those checks fails.
        *from = to.to_string();
        self.verify(file, to)?;
        #[cfg(unix)]
        self.directory.sync_all()?;
        Ok(())
    }
    fn remove(&self, name: &str) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            if let Ok(name) = std::ffi::CString::new(name) {
                unsafe { libc::unlinkat(self.directory.as_raw_fd(), name.as_ptr(), 0) };
            }
        }
        #[cfg(windows)]
        {
            let _ = fs::remove_file(self.path.join(name));
        }
    }
}
#[cfg(windows)]
fn file_identity(file: &File) -> Result<(u32, u32, u32, u32)> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::*;
    let mut info = std::mem::MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::zeroed();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, info.as_mut_ptr()) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let info = unsafe { info.assume_init() };
    Ok((
        info.dwVolumeSerialNumber,
        info.nFileIndexHigh,
        info.nFileIndexLow,
        info.dwFileAttributes,
    ))
}
pub fn source_identity(file: &File) -> Result<String> {
    let metadata = file.metadata()?;
    #[cfg(windows)]
    let identity = {
        let id = file_identity(file)?;
        format!("{}:{}:{}", id.0, id.1, id.2)
    };
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        format!("{}:{}", metadata.dev(), metadata.ino())
    };
    Ok(format!(
        "{identity}:{}:{:?}",
        metadata.len(),
        metadata.modified()?
    ))
}
pub fn same_file(a: &File, b: &File) -> Result<bool> {
    #[cfg(windows)]
    {
        let a = file_identity(a)?;
        let b = file_identity(b)?;
        Ok((a.0, a.1, a.2) == (b.0, b.1, b.2))
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let a = a.metadata()?;
        let b = b.metadata()?;
        Ok(a.dev() == b.dev() && a.ino() == b.ino())
    }
}
#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    a.to_string_lossy()
        .eq_ignore_ascii_case(&b.to_string_lossy())
}
#[cfg(windows)]
fn final_path(file: &File) -> Result<PathBuf> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::GetFinalPathNameByHandleW;
    let mut buffer = vec![0u16; 32768];
    let length = unsafe {
        GetFinalPathNameByHandleW(
            file.as_raw_handle() as _,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            0,
        )
    };
    anyhow::ensure!(
        length > 0 && (length as usize) < buffer.len(),
        "Cannot resolve kernel file-handle path"
    );
    Ok(PathBuf::from(String::from_utf16(
        &buffer[..length as usize],
    )?))
}

/// Staged file. Final destination is never visible until the verified commit.
pub struct PendingOutput {
    target: PathBuf,
    current: String,
    file: Option<File>,
    directory: DirectoryGuard,
    committed: bool,
    preserve: bool,
}
impl PendingOutput {
    pub fn new(source: &str, directory: &Path, filename: &str) -> Result<Self> {
        let target = output_path(source, directory, filename)?;
        anyhow::ensure!(
            !target.exists(),
            "Refusing to overwrite an existing output file"
        );
        let directory = DirectoryGuard::open(target.parent().context("Missing output directory")?)?;
        let current = format!("{filename}.part");
        let file = directory
            .create(source, &current)
            .context("Refusing to overwrite an existing .part file")?;
        Ok(Self {
            target,
            current,
            file: Some(file),
            directory,
            committed: false,
            preserve: false,
        })
    }
    pub fn file(&mut self) -> Result<&mut File> {
        self.file.as_mut().context("Output already closed")
    }
    pub fn final_path(&self) -> &Path {
        &self.target
    }
    pub fn part_path(&self) -> PathBuf {
        self.directory.path.join(&self.current)
    }
    pub fn preserve_on_error(&mut self) {
        self.preserve = true;
    }
    pub fn preallocate(&mut self, size: u64) -> Result<()> {
        anyhow::ensure!(
            size <= i64::MAX as u64,
            "Allocation size exceeds platform limit"
        );
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::*;
            let info = FILE_ALLOCATION_INFO {
                AllocationSize: size as i64,
            };
            if unsafe {
                SetFileInformationByHandle(
                    self.file()?.as_raw_handle() as _,
                    FileAllocationInfo,
                    (&info as *const FILE_ALLOCATION_INFO).cast(),
                    std::mem::size_of::<FILE_ALLOCATION_INFO>() as u32,
                )
            } == 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
        }
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            #[cfg(target_os = "linux")]
            {
                let result = unsafe {
                    libc::posix_fallocate(self.file()?.as_raw_fd(), 0, size as libc::off_t)
                };
                if result != 0 {
                    return Err(std::io::Error::from_raw_os_error(result).into());
                }
            }
            #[cfg(not(target_os = "linux"))]
            {
                anyhow::bail!(
                    "NotImplemented: physical mapfile preallocation on this Unix platform"
                );
            }
        }
        self.file()?.set_len(size)?;
        Ok(())
    }
    fn sync(&mut self) -> Result<()> {
        self.file()?.sync_all()?;
        Ok(())
    }
    fn rename_final(&mut self) -> Result<()> {
        let name = self
            .target
            .file_name()
            .and_then(|n| n.to_str())
            .context("Invalid output filename")?
            .to_string();
        self.directory.rename(
            self.file.as_ref().context("Output closed")?,
            &mut self.current,
            &name,
        )?;
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        let name = format!(
            "{}.part",
            self.target
                .file_name()
                .and_then(|n| n.to_str())
                .context("Invalid output filename")?
        );
        if self.current == name {
            return Ok(());
        }
        self.directory.rename(
            self.file.as_ref().context("Output closed")?,
            &mut self.current,
            &name,
        )?;
        Ok(())
    }
}
impl Drop for PendingOutput {
    fn drop(&mut self) {
        self.file.take();
        if !self.committed && !self.preserve {
            self.directory.remove(&self.current);
        }
    }
}

/// Two-file protocol: sync both; publish metadata first; publish data last.
/// A failed data rename rolls metadata back and preserves both staged files.
pub fn commit_pair(data: PendingOutput, metadata: PendingOutput) -> Result<(PathBuf, PathBuf)> {
    commit_pair_with(data, metadata, || Ok(()))
}
fn commit_pair_with(
    mut data: PendingOutput,
    mut metadata: PendingOutput,
    before_data: impl FnOnce() -> std::io::Result<()>,
) -> Result<(PathBuf, PathBuf)> {
    data.preserve_on_error();
    metadata.preserve_on_error();
    let outcome = (|| -> Result<()> {
        data.sync()?;
        metadata.sync()?;
        metadata.rename_final()?;
        before_data()?;
        data.rename_final()?;
        Ok(())
    })();
    if let Err(error) = outcome {
        // Attempt both rollbacks even if one fails. Never hide the original
        // failure or report stale .part paths after a successful kernel rename.
        let data_rollback = data.rollback().err();
        let metadata_rollback = metadata.rollback().err();
        anyhow::bail!(
            "Commit failed ({}); recovery required: data={}, metadata={}; rollback data={:?}, metadata={:?}",
            error.root_cause(), data.part_path().display(), metadata.part_path().display(),
            data_rollback, metadata_rollback
        );
    }
    data.committed = true;
    metadata.committed = true;
    Ok((data.target.clone(), metadata.target.clone()))
}

#[cfg(test)]
fn create_output(source: &str, directory: &Path, name: &str) -> Result<PendingOutput> {
    PendingOutput::new(source, directory, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    #[test]
    fn swapped_parent_symlink_is_rejected_before_writing() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!("resc-symlink-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let original = root.join("output");
        let elsewhere = root.join("elsewhere");
        fs::create_dir(&original).unwrap();
        fs::create_dir(&elsewhere).unwrap();
        let source = root.join("source.img");
        fs::write(&source, b"source").unwrap();
        let guard = DirectoryGuard::open(&original).unwrap();
        let moved = root.join("moved");
        fs::rename(&original, &moved).unwrap();
        symlink(&elsewhere, &original).unwrap();
        assert!(guard.create(source.to_str().unwrap(), "file.part").is_err());
        assert!(!elsewhere.join("file.part").exists());
        drop(guard);
        fs::remove_file(original).unwrap();
        fs::remove_file(moved.join("file.part")).unwrap();
        fs::remove_file(source).unwrap();
        fs::remove_dir(moved).unwrap();
        fs::remove_dir(elsewhere).unwrap();
        fs::remove_dir(root).unwrap();
    }
    #[test]
    fn quota_failure_during_data_or_map_write_preserves_staged_evidence() {
        use std::io::{self, Write};
        struct Quota<'a> {
            file: &'a mut File,
            remaining: usize,
        }
        impl Write for Quota<'_> {
            fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
                if self.remaining == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::StorageFull,
                        "injected quota exhausted",
                    ));
                }
                let count = bytes.len().min(self.remaining);
                let written = self.file.write(&bytes[..count])?;
                self.remaining -= written;
                Ok(written)
            }
            fn flush(&mut self) -> io::Result<()> {
                self.file.flush()
            }
        }
        for failing in ["data", "map"] {
            let dir = std::env::temp_dir().join(format!("resc-quota-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&dir).unwrap();
            let source = dir.join("source.img");
            fs::write(&source, [0u8; 512]).unwrap();
            let mut data = PendingOutput::new(source.to_str().unwrap(), &dir, "clone.img").unwrap();
            let mut map = PendingOutput::new(source.to_str().unwrap(), &dir, "clone.map").unwrap();
            data.preserve_on_error();
            map.preserve_on_error();
            let file = if failing == "data" {
                data.file().unwrap()
            } else {
                map.file().unwrap()
            };
            let error = Quota {
                file,
                remaining: 10000,
            }
            .write_all(&[0x42; 20000])
            .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::StorageFull);
            drop(data);
            drop(map);
            assert!(!dir.join("clone.img").exists() && !dir.join("clone.map").exists());
            let partial = if failing == "data" {
                "clone.img.part"
            } else {
                "clone.map.part"
            };
            assert_eq!(fs::metadata(dir.join(partial)).unwrap().len(), 10000);
            assert_eq!(fs::read(&source).unwrap(), [0u8; 512]);
            for name in ["source.img", "clone.img.part", "clone.map.part"] {
                fs::remove_file(dir.join(name)).unwrap();
            }
            fs::remove_dir(dir).unwrap();
        }
    }
    #[test]
    fn storage_full_during_second_commit_rolls_metadata_back_and_preserves_parts() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("resc-pair-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source.img");
        fs::write(&source, [0u8; 512]).unwrap();
        let source = source.to_str().unwrap();
        let mut image = PendingOutput::new(source, &dir, "clone.img").unwrap();
        let mut map = PendingOutput::new(source, &dir, "clone.map").unwrap();
        image.file().unwrap().write_all(b"copied evidence").unwrap();
        map.file().unwrap().write_all(b"mapfile").unwrap();
        let error = commit_pair_with(image, map, || {
            Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "injected ENOSPC",
            ))
        })
        .unwrap_err();
        assert!(error.to_string().contains("recovery required"));
        assert!(!dir.join("clone.img").exists());
        assert!(!dir.join("clone.map").exists());
        assert_eq!(
            fs::read(dir.join("clone.img.part")).unwrap(),
            b"copied evidence"
        );
        assert_eq!(fs::read(dir.join("clone.map.part")).unwrap(), b"mapfile");
        for name in ["source.img", "clone.img.part", "clone.map.part"] {
            fs::remove_file(dir.join(name)).unwrap();
        }
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn pinned_directory_and_handle_identity_are_verified_before_writing() {
        let dir = std::env::temp_dir().join(format!("resc-handle-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("source.img");
        fs::write(&source, b"evidence").unwrap();
        let pending = PendingOutput::new(source.to_str().unwrap(), &dir, "result.bin").unwrap();
        assert!(!same_file(
            &File::open(&source).unwrap(),
            pending.file.as_ref().unwrap()
        )
        .unwrap());
        pending
            .directory
            .verify(pending.file.as_ref().unwrap(), &pending.current)
            .unwrap();
        drop(pending);
        assert!(!dir.join("result.bin.part").exists());
        fs::remove_file(source).unwrap();
        fs::remove_dir(dir).unwrap();
    }
    #[test]
    fn source_collision_and_existing_outputs_are_preserved() {
        let dir = std::env::temp_dir().join(format!("resc-safety-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        let source = dir.join("recovered_file.bin");
        fs::write(&source, b"evidence").unwrap();
        let source_str = source.to_str().unwrap();
        assert!(create_output(source_str, &dir, "recovered_file.bin").is_err());
        assert!(create_output(source_str, &source.join("nested"), "x").is_err());
        let existing = dir.join("existing.bin");
        fs::write(&existing, b"keep").unwrap();
        assert!(create_output(source_str, &dir, "existing.bin").is_err());
        assert_eq!(fs::read(&source).unwrap(), b"evidence");
        assert_eq!(fs::read(&existing).unwrap(), b"keep");
        fs::remove_file(source).unwrap();
        fs::remove_file(existing).unwrap();
        fs::remove_dir(dir).unwrap();
    }
}
