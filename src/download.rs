use crate::state::AppState;
use crate::tokens::verify;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use std::fs::{File as StdFile, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path as StdPath, PathBuf};
use tokio::fs::File;

/// Resolve the real on-disk path of an open file descriptor.
///
/// On macOS, uses `fcntl(F_GETPATH)` which queries the kernel for the path
/// the fd was opened with. On Linux, canonicalizes `/dev/fd/N` which is a
/// synthetic symlink to the real file.
///
/// In both cases, the returned path reflects the inode the fd is bound to,
/// not the current state of any filesystem path.
fn fd_real_path(file: &StdFile) -> Result<PathBuf, StatusCode> {
    #[cfg(target_os = "macos")]
    {
        let fd = file.as_raw_fd();
        let mut buf = [0u8; libc::PATH_MAX as usize];
        let ret = unsafe { libc::fcntl(fd, libc::F_GETPATH, buf.as_mut_ptr()) };
        if ret == -1 {
            return Err(StatusCode::GONE);
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
        StdPath::new(&fd_link)
            .canonicalize()
            .map_err(|_| StatusCode::GONE)
    }
}

/// Open a file with O_NOFOLLOW and derive its real path from the fd.
///
/// Returns the opened file handle and its canonical path (resolved from the
/// fd, not the input string). The caller must still perform a `starts_with`
/// check against the allowed base directory.
fn open_served_path(path: &StdPath) -> Result<(StdFile, PathBuf), StatusCode> {
    // O_NOFOLLOW rejects the open if the final component is a symlink.
    // This is a single syscall so there is no window between check and use.
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| {
            if e.raw_os_error() == Some(libc::ELOOP) {
                tracing::warn!("O_NOFOLLOW rejected symlink at: {}", path.display());
            }
            StatusCode::GONE
        })?;

    // Derive the canonical path from the fd itself, not from the input path.
    // This closes the intermediate-directory symlink TOCTOU: even if an
    // attacker swaps an intermediate component, the fd still points to the
    // inode we opened, and the path we check is the real destination.
    let canonical = fd_real_path(&file)?;

    Ok((file, canonical))
}

pub async fn download(
    State(state): State<AppState>,
    Path(tok): Path<String>,
) -> Result<DownloadResponder, StatusCode> {
    // Verify the token and extract the filepath from its payload
    let file_path = match verify(&tok, &state.config.token_key) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("Invalid download token: {}", e);
            return Err(StatusCode::GONE);
        }
    };

    // If file_path starts with "static/", strip it
    let cleaned_file_path = file_path
        .strip_prefix("static/")
        .or_else(|| file_path.strip_prefix("static"))
        .unwrap_or(&file_path);
    let full_path = StdPath::new(&state.config.static_dir).join(cleaned_file_path);

    let (std_file, canonical) = open_served_path(&full_path)?;

    let download_base = StdPath::new(&state.config.static_dir)
        .canonicalize()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    if !canonical.starts_with(&download_base) {
        return Err(StatusCode::GONE);
    }

    // Extract filename from path
    let filename = StdPath::new(&file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("download")
        .to_string();

    // File size from the fd's metadata — no seek needed.
    let size = std_file.metadata().map(|m| m.len()).map_err(|e| {
        tracing::error!("Failed to read file metadata: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    // Serve from the already-opened fd — never re-open, so no TOCTOU window.
    let file = File::from_std(std_file);

    let content_type = canonical
        .extension()
        .and_then(|e| e.to_str())
        .map(|ext| {
            mime_guess::from_ext(ext)
                .first_or_octet_stream()
                .to_string()
        })
        .unwrap_or_else(|| "application/octet-stream".to_string());

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
    content_type: String,
    size: u64,
}

impl IntoResponse for DownloadResponder {
    fn into_response(self) -> Response {
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

        let disposition = format!(
            "attachment; filename=\"{}\"; filename*=UTF-8''{}",
            ascii_filename, encoded_filename
        );

        let stream = tokio::io::BufReader::new(self.file);
        let body = axum::body::Body::from_stream(tokio_util::io::ReaderStream::new(stream));

        axum::response::Response::builder()
            .header(axum::http::header::CONTENT_TYPE, self.content_type)
            .header(axum::http::header::CONTENT_DISPOSITION, disposition)
            .header(axum::http::header::CONTENT_LENGTH, self.size.to_string())
            .body(body)
            .unwrap()
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
    fn try_serve_from_path(path: &StdPath, base: &StdPath) -> Result<(), StatusCode> {
        let (_file, canonical) = open_served_path(path)?;
        let base_canonical = base.canonicalize().map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
        if canonical.starts_with(&base_canonical) {
            Ok(())
        } else {
            Err(StatusCode::GONE)
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

        assert_eq!(try_serve_from_path(&link, &base), Err(StatusCode::GONE));
    }

    #[test]
    fn nonexistent_file_returns_gone() {
        let tmp = TempDir::new().unwrap();
        let base = tmp.path().join("base");
        fs::create_dir(&base).unwrap();

        assert_eq!(
            try_serve_from_path(&base.join("nope.txt"), &base),
            Err(StatusCode::GONE)
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
        assert_eq!(try_serve_from_path(&file, &base), Err(StatusCode::GONE));
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
        assert_eq!(try_serve_from_path(&path, &base), Err(StatusCode::GONE));
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
