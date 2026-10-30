//! Bounded snapshots of explicitly selected local files, independent of the store worker.

use mailstronaut_mail::compose::{self, AttachmentData};
use std::{fs::File, io::Read as _, path::PathBuf};

/// Copy selected regular files into bounded owned snapshots, without retaining paths.
/// Call from a blocking task; this function performs filesystem I/O.
/// # Errors
/// Returns a static redacted explanation for access, name, type, size or change failures.
pub fn read_files(paths: Vec<PathBuf>, remaining: usize) -> Result<Vec<AttachmentData>, String> {
    if paths.len() > compose::MAX_ATTACHMENTS {
        return Err("Too many attachments (maximum 16).".into());
    }
    let mut files = Vec::new();
    let mut available = remaining.min(compose::MAX_ATTACHMENT_BYTES);
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| compose::valid_attachment_name(name))
            .ok_or("Choose files with valid UTF-8 basenames (maximum 255 bytes).")?
            .to_owned();
        // NONBLOCK avoids hanging the worker on a FIFO or device selected by mistake.
        let file = File::from(
            rustix::fs::open(
                &path,
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::NONBLOCK
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| "Cannot read a selected file. No files were attached.")?,
        );
        let before = file
            .metadata()
            .map_err(|_| "Cannot inspect a selected file.")?;
        if !before.is_file() {
            return Err("Only regular files can be attached.".into());
        }
        if before.len() > available as u64 {
            return Err("Attachments exceed the 5 MiB total limit.".into());
        }
        let mut bytes = Vec::new();
        (&file)
            .take(available as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read a selected file. No files were attached.")?;
        let after = file
            .metadata()
            .map_err(|_| "Cannot inspect a selected file.")?;
        if bytes.len() > available
            || before.len() != after.len()
            || before.modified().ok() != after.modified().ok()
            || bytes.len() as u64 != after.len()
        {
            return Err(
                "A selected file changed or exceeded the size limit. Select it again.".into(),
            );
        }
        available -= bytes.len();
        files.push(AttachmentData { name, bytes });
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_binary_unicode_and_empty_files_without_retaining_paths() {
        let root = tempfile::tempdir().expect("fixture");
        let binary = root.path().join("résumé-日本語.bin");
        let empty = root.path().join("empty");
        std::fs::write(&binary, [0, 255, 1, 0]).expect("binary");
        std::fs::write(&empty, []).expect("empty");
        let files = read_files(vec![binary.clone(), empty], 4).expect("bounded files");
        std::fs::remove_file(binary).expect("source removed");
        assert_eq!(files[0].bytes, [0, 255, 1, 0]);
        assert_eq!(files[0].name, "résumé-日本語.bin");
        assert_eq!(files[1].bytes, [] as [u8; 0]);
        assert!(!format!("{files:?}").contains("résumé"));
        assert!(read_files(vec![root.path().into()], 4).is_err());
    }
    #[test]
    fn rejects_aggregate_oversize_and_missing_sources_as_one_failed_import() {
        let root = tempfile::tempdir().expect("fixture");
        let a = root.path().join("a");
        let b = root.path().join("b");
        std::fs::write(&a, [1, 2, 3]).expect("a");
        std::fs::write(&b, [4, 5, 6]).expect("b");
        assert!(read_files(vec![a.clone(), b], 5).is_err());
        assert!(read_files(vec![a, root.path().join("missing")], 10).is_err());
    }
}
