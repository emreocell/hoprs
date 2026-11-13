//! TCP settings shared by every connection (SSH here, FTP in `hoprs-remote`).
//!
//! TCP keepalive makes the kernel send a probe after [`TCP_KEEPALIVE_IDLE`] without
//! traffic, then every [`TCP_KEEPALIVE_INTERVAL`], and give up after
//! [`TCP_KEEPALIVE_RETRIES`] unanswered probes. Routers and NATs see traffic on an idle
//! connection (so they keep its mapping), and a peer that vanished without a FIN or RST
//! turns into a socket error within about 90 s instead of never.

use std::io;
use std::time::Duration;

use socket2::{SockRef, TcpKeepalive};
use tokio::net::TcpStream;

/// Idle time before the first keepalive probe.
pub const TCP_KEEPALIVE_IDLE: Duration = Duration::from_secs(60);
/// Time between unanswered probes.
pub const TCP_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);
/// Unanswered probes before the connection is dropped.
pub const TCP_KEEPALIVE_RETRIES: u32 = 3;

/// Turns on TCP keepalive with the values above.
pub fn set_tcp_keepalive(stream: &TcpStream) -> io::Result<()> {
    let keepalive = TcpKeepalive::new()
        .with_time(TCP_KEEPALIVE_IDLE)
        .with_interval(TCP_KEEPALIVE_INTERVAL)
        .with_retries(TCP_KEEPALIVE_RETRIES);
    SockRef::from(stream).set_tcp_keepalive(&keepalive)
}

/// What a socket's TCP keepalive is set to (tests).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeepaliveSettings {
    pub enabled: bool,
    /// `None` where the platform cannot read it back.
    pub idle: Option<Duration>,
    pub interval: Option<Duration>,
    pub retries: Option<u32>,
}

impl KeepaliveSettings {
    /// The settings [`set_tcp_keepalive`] applies, as they read back on this platform.
    pub fn expected() -> Self {
        let readable = cfg!(any(target_os = "linux", target_os = "macos"));
        Self {
            enabled: true,
            idle: readable.then_some(TCP_KEEPALIVE_IDLE),
            interval: readable.then_some(TCP_KEEPALIVE_INTERVAL),
            retries: readable.then_some(TCP_KEEPALIVE_RETRIES),
        }
    }
}

/// Reads a socket's TCP keepalive settings back (tests).
pub fn tcp_keepalive_of(stream: &TcpStream) -> io::Result<KeepaliveSettings> {
    let sock = SockRef::from(stream);
    let enabled = sock.keepalive()?;
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    let (idle, interval, retries) = (
        Some(sock.tcp_keepalive_time()?),
        Some(sock.tcp_keepalive_interval()?),
        Some(sock.tcp_keepalive_retries()?),
    );
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let (idle, interval, retries) = (None, None, None);
    Ok(KeepaliveSettings {
        enabled,
        idle,
        interval,
        retries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn keepalive_values_read_back() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let stream = TcpStream::connect(listener.local_addr().unwrap())
            .await
            .unwrap();
        assert!(!tcp_keepalive_of(&stream).unwrap().enabled);
        set_tcp_keepalive(&stream).unwrap();
        assert_eq!(
            tcp_keepalive_of(&stream).unwrap(),
            KeepaliveSettings::expected()
        );
    }
}
