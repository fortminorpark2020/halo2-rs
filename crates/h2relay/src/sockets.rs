//! The UDP sockets both ends use: non-blocking, so a send never waits (a
//! full buffer drops the datagram, as the network would), with a wait
//! for something to read that also serves as the timer.

use socket2::{Domain, Protocol, Socket, Type};
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, UdpSocket};
use std::time::Duration;

/// A non-blocking UDP socket bound to `addr`, with `buffers` bytes of
/// send and receive buffer if the system allows that much.
pub(crate) fn bind(addr: SocketAddr, buffers: usize) -> io::Result<UdpSocket> {
    let socket = Socket::new(Domain::for_address(addr), Type::DGRAM, Some(Protocol::UDP))?;
    if addr.is_ipv6() && addr.ip().is_unspecified() {
        // IPv4 too, where the system allows it.
        let _ = socket.set_only_v6(false);
    }
    // Only a request: the system may give less.
    let _ = socket.set_recv_buffer_size(buffers);
    let _ = socket.set_send_buffer_size(buffers);
    socket.bind(&addr.into())?;
    socket.set_nonblocking(true)?;
    Ok(socket.into())
}

/// Wait until `socket` has something to read, or `timeout` passes.
pub(crate) fn wait(socket: &UdpSocket, timeout: Duration) {
    use rustix::event::{PollFd, PollFlags, Timespec};
    let mut fds = [PollFd::new(socket, PollFlags::IN)];
    let timeout = Timespec::try_from(timeout).ok();
    if rustix::event::poll(&mut fds, timeout.as_ref()).is_err() {
        // Not to spin, whatever it was (an interrupt, most likely).
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// What a failed receive means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecvError {
    /// Nothing more to read now.
    Empty,
    /// One datagram's worth of trouble, already gone: try the next. Windows
    /// reports an ICMP error about an earlier send (port or host
    /// unreachable, time to live expired) on a later receive, and a
    /// datagram too big for the buffer as an error.
    Skip,
    /// Something else: wait a moment before trying again.
    Other,
}

/// Winsock codes that are about one earlier datagram: WSAEMSGSIZE (it
/// didn't fit, and the rest was dropped), WSAENETUNREACH, WSAENETRESET
/// (on a datagram socket: its time to live expired) and WSAEHOSTUNREACH.
const WINDOWS_SKIP: [i32; 4] = [10040, 10051, 10052, 10065];

pub(crate) fn recv_error(e: &io::Error) -> RecvError {
    match e.kind() {
        ErrorKind::WouldBlock | ErrorKind::TimedOut => RecvError::Empty,
        ErrorKind::ConnectionReset
        | ErrorKind::ConnectionRefused
        | ErrorKind::HostUnreachable
        | ErrorKind::NetworkUnreachable
        | ErrorKind::Interrupted => RecvError::Skip,
        _ if cfg!(windows) && e.raw_os_error().is_some_and(|c| WINDOWS_SKIP.contains(&c)) => {
            RecvError::Skip
        }
        _ => RecvError::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_about_one_datagram_are_skipped() {
        let kind = |kind: ErrorKind| recv_error(&io::Error::from(kind));
        assert_eq!(kind(ErrorKind::WouldBlock), RecvError::Empty);
        for skip in [
            ErrorKind::ConnectionReset,
            ErrorKind::ConnectionRefused,
            ErrorKind::HostUnreachable,
            ErrorKind::NetworkUnreachable,
            ErrorKind::Interrupted,
        ] {
            assert_eq!(kind(skip), RecvError::Skip, "{skip:?}");
        }
        assert_eq!(kind(ErrorKind::PermissionDenied), RecvError::Other);
        // Winsock's own codes, on Windows only (they mean other things
        // elsewhere, or nothing).
        let skip = if cfg!(windows) {
            RecvError::Skip
        } else {
            RecvError::Other
        };
        for code in WINDOWS_SKIP {
            let e = io::Error::from_raw_os_error(code);
            if !matches!(
                e.kind(),
                ErrorKind::HostUnreachable | ErrorKind::NetworkUnreachable
            ) {
                assert_eq!(recv_error(&e), skip, "{code}");
            }
        }
    }
}
