use crate::config::Config;
use crate::tokens::verify;
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rocket::http::{ContentType, Status};
use rocket::response::{self, Responder, Response};
use rocket::{Request, State};
use std::fs::{File as StdFile, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use tokio::fs::File;

/// Resolve the real on-disk path of an open file descriptor.
///
/// On macOS, uses `fcntl(F_GETPATH)` which queries the kernel for the path
/// the fd was opened with. On Linux, canonicalizes `/dev/fd/N` which is a
/// synthetic symlink to the real file.
///
/// In both cases, the returned path reflects the inode the fd is bound to,
/// not the current state of any filesystem path.
fn fd_real_path(file: &StdFile) -> Result<PathBuf, Status> {
    #[cfg(target_os = "macos")]
    {
        let fd = file.as_raw_fd();
        let mut buf = [0u8; libc::PATH_MAX as usize];
        let ret = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
        if ret == -1 {
            return Err(Status::Gone);
        }
        // fcntl(F_GETPATH) writes a NUL-terminated C string.
        let nul = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
        let path = std::ffi::OsStr::from_bytes(&buf[..nul]);
        Ok(PathBuf::from(path))
    }
    #[cfg(target_os = "linux")]
    {
        let fd = file.as_raw_fd();
        let fd_link = format!("/dev/fd/{}", fd);
        Path::new(&fd_link)
            .canonicalize()
            .map_err(|_| Status::Gone)
    }
}

/// Open a file with O_NOFOLLOW and derive its real path from the fd.
///
/// Returns the opened file handle and its canonical path (resolved from the
/// fd, not the input string). The caller must still perform a `starts_with`
/// check against the allowed base directory.
fn open_served_path(path: &Path) -> Result<(StdFile, PathBuf), Status> {
    // O_NOFOLLOW rejects the open if the final component is a symlink.
    // This is a single syscall so there is no window between check and use.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) {
                rocket::warn!("O_NOFOLLOW rejected symlink at: {}", path.display());
            }
            Status::Gone
        })?;

    // Derive the canonical path from the fd itself, not from the input path.
    // This closes the intermediate-directory symlink TOCTOU: even if an
    // attacker swaps an intermediate component, the fd still points to the
    // inode we opened, and the path we check is the real destination.
    let canonical = fd_real_path(&file)?;

    Ok((file, canonical))
}

#[get("/download/<tok>")]
pub async fn download(config: &State<Config>, tok: &str) -> Result<DownloadResponder, Status> {
    // Verify the token and extract the filepath from its payload
    let file_path = match verify(tok, &config.token_key) {
        Ok(p) => p,
        Err(e) => {
            rocket::warn!("Invalid download token: {}", e);
            return Err(Status::Gone);
        }
    };

    // If file_path starts with "static/", strip it
    let cleaned_file_path = file_path
        .strip_prefix("static/")
        .or_else(|| file_path.strip_prefix("static"))
        .unwrap_or(&file_path);
    let full_path = Path::new(&config.static_dir).join(cleaned_file_path);

    let (std_file, canonical) = open_served_path(&full_path)?;

    let download_base = Path::new(&config.static_dir)
        .canonicalize()
        .map_err(|_| Status::InternalServerError)?;

    if !canonical.starts_with(&download_base) {
        return Err(Status::Gone);
    }

    // Extract filename from path
    let filename = Path::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();

    // File size from the fd's metadata — no seek needed.
    let size = std_file.metadata().map(|m| m.len()).map_err(|e| {
        rocket::error!("Failed to read file metadata: {:?}", e);
        Status::InternalServerError
    })?;

    // Serve from the already-opened fd — never re-open, so no TOCTOU window.
    let file = File::from_std(std_file);

    let content_type = canonical
        .extension()
        .and_then(|e| e.to_str())
        .and_then(ContentType::from_extension)
        .unwrap_or(ContentType::Binary);

    Ok(DownloadResponder {
        file,
        filename,
        content_type,
        size,
    })
}

#[derive(Debug)]
pub struct DownloadResponder {
    file: File,
    filename: String,
    content_type: ContentType,
    size: u64,
}

impl<'r> Responder<'r, 'static> for DownloadResponder {
    fn respond_to(self, req: &'r Request<'_>) -> response::Result<'static> {
        // Use RFC 6266 format with percent-encoding for the filename* parameter
        let encoded_filename = utf8_percent_encode(&self.filename, NON_ALPHANUMERIC).to_string();

        // ASCII fallback replaces non-ASCII chars with underscores
        let ascii_filename: String = self
            .filename
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        Response::build_from(self.file.respond_to(req)?)
            .header(self.content_type)
            .raw_header(
                "Content-Disposition",
                format!(
                    "attachment; filename=\"{}\"; filename*=UTF-8''{}",
                    ascii_filename, encoded_filename
                ),
            )
            .raw_header("Content-Length", self.size.to_string())
            .ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::TempDir;

    /// Mirror of the security logic in `download()`: open with O_NOFOLLOW,
    /// resolve the canonical path from the fd, and check it's inside base.
    /// Returns Ok(()) only when the full pipeline accepts the path.
    fn try_serve_from_path(path: &Path, base: &Path) -> Result<(), Status> {
        let (_file, canonical) = open_served_path(path)?;
        let base_canonical = base.canonicalize().map_err(|_| Status::InternalServerError)?;
        if canonical.starts_with(&base_canonical) {
            Ok(())
        } else {
            Err(Status::Gone)
        }
    }

    #[test]
    fn regular_file_inside_base_accepted() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        fs::create_dir(&base).unwrap();
        let file = base.join("test.txt");
        fs::write(&file, "hello").unwrap();

        assert!(try_serve_from_path(&file, &base).is_ok());
    }

    #[test]
    fn direct_symlink_rejected() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        fs::create_dir(&base).unwrap();
        let file = base.join("real.txt");
        fs::write(&file, "hello").unwrap();
        let link = base.join("link.txt");
        symlink(&file, &link).unwrap();

        assert_eq!(try_serve_from_path(&link, &base), Err(Status::Gone));
    }

    #[test]
    fn nonexistent_file_returns_gone() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        fs::create_dir(&base).unwrap();

        assert_eq!(
            try_serve_from_path(&base.join("nope.txt"), &base),
            Err(Status::Gone)
        );
    }

    #[test]
    fn file_outside_base_rejected() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        let sibling = tmp.path().join("sibling");
        fs::create_dir(&base).unwrap();
        fs::create_dir(&sibling).unwrap();
        let file = sibling.join("secret.txt");
        fs::write(&file, "secret").unwrap();

        // The open itself succeeds (regular file), but the bounds check rejects.
        assert!(open_served_path(&file).is_ok(), "open should succeed for a regular file outside base");
        assert_eq!(try_serve_from_path(&file, &base), Err(Status::Gone));
    }

    #[test]
    fn intermediate_symlink_outside_base_caught_by_fd_path() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&base).unwrap();
        fs::create_dir_all(&outside).unwrap();

        let secret = outside.join("secret.txt");
        fs::write(&secret, "secret").unwrap();

        // Intermediate directory inside base is a symlink to outside/.
        let link_dir = base.join("link_dir");
        symlink(&outside, &link_dir).unwrap();

        // The final component "secret.txt" is a regular file, so O_NOFOLLOW
        // doesn't reject the open. The fd-based canonical path should resolve
        // through the intermediate symlink to outside/secret.txt, which is
        // outside base — caught by the bounds check.
        let path = link_dir.join("secret.txt");
        assert_eq!(try_serve_from_path(&path, &base), Err(Status::Gone));
    }

    #[test]
    fn fd_path_matches_real_file() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        fs::create_dir(&base).unwrap();
        let file = base.join("test.txt");
        fs::write(&file, "hello").unwrap();

        let (f, canonical) = open_served_path(&file).unwrap();
        assert_eq!(canonical, file.canonicalize().unwrap());

        use std::io::Read;
        let mut contents = String::new();
        f.take(u64::MAX).read_to_string(&mut contents).unwrap();
        assert_eq!(contents, "hello");
    }
}
