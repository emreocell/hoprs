//! Resumable, cancellable file copy between two `RemoteFs`.

use std::sync::atomic::{AtomicU64, Ordering};

use hoprs_core::{Error, ErrorKind, RemoteEntry, RemotePath, Result};
use hoprs_remote::RemoteFs;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

/// How a `copy_file` call ended.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CopyOutcome {
    Completed,
    Cancelled,
}

/// Copies `src_entry` to `dst_path`, starting at `offset`.
///
/// `progress` holds the absolute byte position (`offset` + bytes copied).
/// Cancellation is checked before each block read.
#[allow(clippy::too_many_arguments)]
pub async fn copy_file(
    src: &dyn RemoteFs,
    src_entry: &RemoteEntry,
    dst: &dyn RemoteFs,
    dst_path: &RemotePath,
    offset: u64,
    buf_size: usize,
    progress: &AtomicU64,
    cancel: &CancellationToken,
) -> Result<CopyOutcome> {
    let mut reader = src.open_read(&src_entry.path, offset).await?;
    let mut writer = dst.open_write(dst_path, offset).await?;
    let mut buf = vec![0u8; buf_size.max(1)];
    let mut copied = 0u64;
    progress.store(offset, Ordering::Relaxed);

    loop {
        if cancel.is_cancelled() {
            let _ = writer.shutdown().await;
            return Ok(CopyOutcome::Cancelled);
        }
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        writer.write_all(&buf[..n]).await?;
        copied += n as u64;
        progress.store(offset + copied, Ordering::Relaxed);
    }
    writer.shutdown().await?;

    let got = dst.stat(dst_path).await?.size;
    if got != src_entry.size {
        return Err(
            Error::new(ErrorKind::Protocol, "Size mismatch after transfer")
                .with_detail(format!("expected {}, got {}", src_entry.size, got)),
        );
    }
    if let Some(mtime) = src_entry.modified {
        let _ = dst.set_modified(dst_path, mtime).await;
    }
    Ok(CopyOutcome::Completed)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use hoprs_remote::{FaultFs, MemFs};

    use super::*;

    fn data(len: usize) -> Vec<u8> {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                (x >> 24) as u8
            })
            .collect()
    }

    fn p(s: &str) -> RemotePath {
        RemotePath::new(s).unwrap()
    }

    fn mtime() -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)
    }

    #[tokio::test]
    async fn copies_whole_file() {
        let src = MemFs::new();
        let dst = MemFs::new();
        let bytes = data(1 << 20);
        src.add_file("/a.bin", &bytes, mtime());
        let entry = src.stat(&p("/a.bin")).await.unwrap();
        let progress = AtomicU64::new(0);
        let out = copy_file(
            &src,
            &entry,
            &dst,
            &p("/b.bin"),
            0,
            64 * 1024,
            &progress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out, CopyOutcome::Completed);
        assert_eq!(dst.read_file("/b.bin").unwrap(), bytes);
        assert_eq!(progress.load(Ordering::Relaxed), 1 << 20);
        assert_eq!(
            dst.stat(&p("/b.bin")).await.unwrap().modified,
            Some(mtime())
        );
    }

    #[tokio::test]
    async fn copies_zero_byte_file() {
        let src = MemFs::new();
        let dst = MemFs::new();
        src.add_file("/e", b"", mtime());
        let entry = src.stat(&p("/e")).await.unwrap();
        let out = copy_file(
            &src,
            &entry,
            &dst,
            &p("/e2"),
            0,
            1024,
            &AtomicU64::new(0),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(out, CopyOutcome::Completed);
        assert_eq!(dst.read_file("/e2").unwrap(), b"");
    }

    #[tokio::test]
    async fn resumes_from_offset() {
        let src = MemFs::new();
        let dst = MemFs::new();
        let bytes = data(1 << 20);
        src.add_file("/a.bin", &bytes, mtime());
        dst.add_file("/b.bin", &bytes[..400_000], mtime());
        let entry = src.stat(&p("/a.bin")).await.unwrap();
        let progress = AtomicU64::new(0);
        copy_file(
            &src,
            &entry,
            &dst,
            &p("/b.bin"),
            400_000,
            64 * 1024,
            &progress,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(dst.read_file("/b.bin").unwrap(), bytes);
        assert_eq!(progress.load(Ordering::Relaxed), 1 << 20);
    }

    #[tokio::test]
    async fn cancel_stops_early() {
        let mem = MemFs::new();
        let bytes = data(1 << 20);
        mem.add_file("/a.bin", &bytes, mtime());
        let src = FaultFs::new(mem);
        src.set_latency(Duration::from_millis(5));
        let dst = MemFs::new();
        let entry = src.stat(&p("/a.bin")).await.unwrap();
        let cancel = CancellationToken::new();
        cancel.cancel();
        let progress = AtomicU64::new(0);
        let out = copy_file(
            &src,
            &entry,
            &dst,
            &p("/b.bin"),
            0,
            64 * 1024,
            &progress,
            &cancel,
        )
        .await
        .unwrap();
        assert_eq!(out, CopyOutcome::Cancelled);
        assert!(progress.load(Ordering::Relaxed) < 1 << 20);
    }

    #[tokio::test]
    async fn io_error_propagates_kind() {
        let mem = MemFs::new();
        mem.add_file("/a.bin", &data(1 << 20), mtime());
        let src = FaultFs::new(mem);
        let dst = MemFs::new();
        let entry = src.stat(&p("/a.bin")).await.unwrap();
        src.fail_io_after(100_000, ErrorKind::Network);
        let err = copy_file(
            &src,
            &entry,
            &dst,
            &p("/b.bin"),
            0,
            64 * 1024,
            &AtomicU64::new(0),
            &CancellationToken::new(),
        )
        .await
        .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Network);
    }
}
