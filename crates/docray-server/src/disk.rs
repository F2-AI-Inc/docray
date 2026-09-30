use std::path::Path;

pub enum RoomError {
    /// Writing would leave less than the configured free-space floor.
    Full,
    /// Free space could not be determined; callers fail closed.
    Io(std::io::Error),
}

/// Checks that writing `incoming` more bytes under `dir` leaves at least
/// `min_free` bytes available to unprivileged users. `min_free == 0` disables
/// the check.
pub fn ensure_room(dir: &Path, min_free: u64, incoming: u64) -> Result<(), RoomError> {
    if min_free == 0 {
        return Ok(());
    }
    let available = available_bytes(dir).map_err(RoomError::Io)?;
    if available.saturating_sub(incoming) < min_free {
        return Err(RoomError::Full);
    }
    Ok(())
}

// statvfs field widths differ by platform (u32 blocks on macOS, u64 on Linux).
#[cfg(unix)]
#[allow(clippy::unnecessary_cast)]
fn available_bytes(dir: &Path) -> std::io::Result<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes())
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;
    // SAFETY: statvfs only writes into the zeroed struct we own, and `path` is
    // a valid NUL-terminated string for the duration of the call.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut stats) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok((stats.f_bavail as u64).saturating_mul(stats.f_frsize as u64))
}

#[cfg(not(unix))]
fn available_bytes(_dir: &Path) -> std::io::Result<u64> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "free-space check needs statvfs; set DOCRAY_MIN_FREE_BYTES=0 to disable it",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_is_checked_against_real_free_space() {
        let dir = std::env::temp_dir();
        let available = available_bytes(&dir).unwrap();
        assert!(available > 0);
        assert!(ensure_room(&dir, 1, 0).is_ok());
        assert!(matches!(
            ensure_room(&dir, u64::MAX, 0),
            Err(RoomError::Full)
        ));
        // Incoming bytes count against the floor: a write larger than all
        // free space can never fit.
        assert!(matches!(
            ensure_room(&dir, 1, available.saturating_add(1)),
            Err(RoomError::Full)
        ));
        assert!(
            ensure_room(&dir, 0, u64::MAX).is_ok(),
            "0 disables the check"
        );
    }

    #[test]
    fn unreadable_path_fails_closed() {
        assert!(matches!(
            ensure_room(Path::new("/nonexistent/docray/data"), 1, 0),
            Err(RoomError::Io(_))
        ));
    }
}
