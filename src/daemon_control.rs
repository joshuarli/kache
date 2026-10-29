//! Kache identity and endpoint adapter for shared lifecycle control.
use super::*;
use crate::lifecycle::Lifecycle;
use kunobi_daemon::local::Duplex;
use kunobi_daemon::{
    ServiceIdentity,
    admission::{Admission, Limits, Pool},
    local::peer::PeerCredentials,
    peer::{Policy, SameUser},
    wire::{self, operation},
};

// Protocol masks: HEALTH | HEALTH_DETAILS | DRAIN, and required health details.
const CONTROL_SUPPORTED: u64 = 0x13;
const CONTROL_REQUIRED: u64 = 0x11;

// Stable service-family UUID. Cache instance separation is independent of builds.
const SERVICE_ID: [u8; 16] = [
    0xa1, 0x94, 0xcb, 0xee, 0x70, 0x11, 0x43, 0x9c, 0xb6, 0x7e, 0x62, 0x96, 0x2a, 0x3a, 0xd8, 0x51,
];

/// The control endpoint, a sibling of the daemon socket.
///
/// Its name is part of the daemon's path budget: a Unix socket address holds
/// 103 bytes of path on macOS and 107 on Linux, and both sockets have to fit.
/// `daemon.ctl` is shorter than the `daemon.sock` it is derived from, so any
/// runtime directory that could hold a daemon socket before this endpoint
/// existed still can. The first name for it, `daemon.control.v2.sock`, cost
/// 11 bytes more and took CI runtime directories that had always worked over
/// the macOS limit (kunobi-ninja/kache#1216).
///
/// Clients and the daemon must derive the same name, so this is the only
/// place that spells it.
pub(super) fn endpoint(config: &Config) -> PathBuf {
    config.socket_path().with_extension("ctl")
}

fn offer(config: &Config) -> Result<wire::Hello> {
    let socket = config.socket_path();
    let parent = socket.parent().context("socket has no parent")?;
    let path = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf())
        .join(socket.file_name().context("socket has no filename")?);
    let instance = blake3::hash(path.as_os_str().as_encoded_bytes())
        .to_hex()
        .to_string();
    let identity = ServiceIdentity::new(SERVICE_ID, "kache", "shared", instance)?;
    Ok(wire::Hello::new(
        &identity,
        CONTROL_SUPPORTED,
        CONTROL_REQUIRED,
    ))
}

pub(super) struct ControlService {
    lifecycle: Arc<Lifecycle>,
    ready: std::sync::atomic::AtomicBool,
    process_id: u32,
    generation: u64,
    build: String,
    revision: u64,
}

impl ControlService {
    pub(super) fn new(
        lifecycle: Arc<Lifecycle>,
        generation: u64,
        build: String,
        revision: u64,
    ) -> Self {
        Self {
            lifecycle,
            ready: std::sync::atomic::AtomicBool::new(false),
            process_id: std::process::id(),
            generation,
            build,
            revision,
        }
    }

    pub(super) fn mark_ready(&self) {
        self.ready
            .store(true, std::sync::atomic::Ordering::Release);
    }

    pub(super) fn snapshot(&self) -> wire::Health {
        let state = self.lifecycle.snapshot();
        wire::Health {
            process_id: self.process_id,
            generation: self.generation,
            build: self.build.clone(),
            revision: self.revision,
            ready: self.ready.load(std::sync::atomic::Ordering::Acquire) && !state.draining,
            draining: state.draining,
            active: state.active as u64,
            ..Default::default()
        }
    }

    pub(super) fn handle(&self, request: &wire::Control) -> std::io::Result<wire::Control> {
        use wire::{MessageKind, operation};
        if request.kind != MessageKind::Lifecycle
            || !request.payload.is_empty()
            || !request.token.is_empty()
            || request.offset.is_some()
            || (request.generation != 0 && request.generation != self.generation)
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "invalid lifecycle request",
            ));
        }
        match request.operation {
            operation::HEALTH => {}
            operation::DRAIN => {
                self.lifecycle.start_drain();
            }
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "unsupported lifecycle request",
                ));
            }
        }
        self.snapshot()
            .response(request)
            .map_err(std::io::Error::other)
    }
}

pub(super) struct Server {
    pub(super) service: Arc<ControlService>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    socket_path: PathBuf,
    _socket: SocketCleanupGuard,
}
impl Server {
    pub(super) fn finish(&mut self) {
        self.stop
            .store(true, std::sync::atomic::Ordering::Release);
        // Wake the blocking accept by connecting once; the loop exits on the
        // stop flag before handling it.
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(&self.socket_path) {
            let _ = stream;
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop
            .store(true, std::sync::atomic::Ordering::Release);
    }
}

pub(super) fn serve(config: &Config, lifecycle: Arc<Lifecycle>) -> Result<Server> {
    let path = endpoint(config);
    let listener = crate::transport::bind_daemon_listener(&path)?
        .context("control endpoint already owned")?;
    listener
        .set_nonblocking(true)
        .context("control listener nonblocking")?;
    let socket = SocketCleanupGuard::new(&path)?;
    // 1.0.x clients replace a daemon whose revision is older than their own
    // executable mtime; later clients compare the release in `build`.
    let service = Arc::new(ControlService::new(
        lifecycle,
        0,
        VERSION.into(),
        build_epoch(),
    ));
    let offer = offer(config)?;
    let handler = Arc::clone(&service);
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopping = Arc::clone(&stop);
    let socket_path = path.clone();
    let thread = std::thread::Builder::new()
        .name("kache-control".to_string())
        .spawn(move || {
            let admission = Arc::new(Admission::new(Limits::default()));
            // The control endpoint is low-traffic; handle each connection to
            // completion on this thread with bounded deadlines.
            loop {
                if stopping.load(std::sync::atomic::Ordering::Acquire) {
                    break;
                }
                match listener.accept() {
                    Ok((stream, _)) => {
                        // The listener is nonblocking so the loop can poll
                        // the stop flag; accepted sockets inherit that mode
                        // on some platforms, but the session handshake below
                        // blocks. Restore blocking mode first.
                        if stream.set_nonblocking(false).is_err() {
                            continue;
                        }
                        let Some(_permit) =
                            admission.try_acquire(Pool::Control)
                        else {
                            continue;
                        };
                        if crate::transport::require_self_peer(
                            crate::transport::peer_euid_std(&stream),
                        )
                        .is_err()
                        {
                            continue;
                        }
                        stream
                            .set_read_timeout(Some(Duration::from_secs(2)))
                            .ok();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(2)))
                            .ok();
                        let mut session = match wire::Session::accept(stream, &offer) {
                            Ok(session) => session,
                            Err(_) => continue,
                        };
                        let request = match session.receive() {
                            Ok(request) => request,
                            Err(_) => continue,
                        };
                        let reply = match handler.handle(&request) {
                            Ok(reply) => reply,
                            Err(_) => continue,
                        };
                        let _ = session.send(&reply);
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock =>
                    {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => {
                        if stopping.load(std::sync::atomic::Ordering::Acquire) {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                }
            }
            let _ = socket_path;
        })
        .context("spawning control server thread")?;
    Ok(Server {
        service,
        stop,
        thread: Some(thread),
        socket_path: path,
        _socket: socket,
    })
}

/// None means no binary endpoint was advertised. Any binary failure is final;
/// callers may retry discovery, but may not downgrade this request to legacy.
pub(super) fn request(
    config: &Config,
    operation: u32,
    deadline: Instant,
) -> Result<Option<wire::Health>> {
    let Some(state) = read_daemon_state(&config.socket_path()) else {
        return Ok(None);
    };
    let Some(version) = state.control_version else {
        return Ok(None);
    };
    anyhow::ensure!(
        version == wire::VERSION,
        "unsupported lifecycle control version {version}"
    );
    let expected = kunobi_daemon::ProcessId::new(state.pid)
        .context("lifecycle advertisement has an invalid PID")?;
    let path = endpoint(config);
    let offer = offer(config)?;
    anyhow::ensure!(
        matches!(operation, operation::HEALTH | operation::DRAIN),
        "unsupported lifecycle operation {operation}"
    );
    let health = {
        let endpoint = crate::transport::daemon_endpoint(&path);
        match operation {
            operation::HEALTH => {
                kunobi_daemon::client::health(&endpoint, &offer, Some(expected), deadline)
            }
            _ => kunobi_daemon::client::drain(&endpoint, &offer, Some(expected), deadline),
        }?
    };
    Ok(Some(health))
}

#[cfg(test)]
fn lifecycle_message(operation: u32) -> wire::Control {
    wire::Control {
        operation,
        ..Default::default()
    }
}

pub(super) fn health(config: &Config, deadline: Instant) -> Result<Option<wire::Health>> {
    request(config, operation::HEALTH, deadline)
}

type Local = kunobi_daemon::local::PlatformDuplex;

fn connect(path: &Path, deadline: Instant) -> Result<Local> {
    let stream = Local::connect_once_until(&crate::transport::daemon_endpoint(path), deadline)?;
    let credentials = stream
        .credentials()
        .context("reading lifecycle peer credentials")?;
    SameUser
        .grant(&credentials)
        .context("authenticating lifecycle peer")?;
    let remaining = deadline.saturating_duration_since(Instant::now());
    anyhow::ensure!(!remaining.is_zero(), "lifecycle deadline expired");
    stream
        .set_read_deadline(Some(remaining))
        .context("setting lifecycle setup deadline")?;
    Ok(stream)
}

/// Bounded compatibility exchange on the existing JSON endpoint. Only callers
/// that found no binary advertisement may use this adapter.
pub(super) fn legacy_request(path: &Path, request: &Request, deadline: Instant) -> Result<String> {
    legacy_exchange(path, request, deadline)
}

fn legacy_exchange(path: &Path, request: &Request, deadline: Instant) -> Result<String> {
    use std::io::{BufRead, Read, Write};
    let stream = connect(path, deadline)?;
    let (read, mut write) = stream.split()?;
    let mut bytes = deser_json::to_string(request).map(String::into_bytes)?;
    bytes.push(b'\n');
    write.write_all(&bytes)?;
    write.flush()?;
    let mut response = String::new();
    std::io::BufReader::new(read.take(65_536)).read_line(&mut response)?;
    if !response.ends_with('\n') {
        return Err(std::io::Error::from(if response.len() == 65_536 {
            std::io::ErrorKind::InvalidData
        } else {
            std::io::ErrorKind::UnexpectedEof
        })
        .into());
    }
    anyhow::ensure!(Instant::now() < deadline, "late legacy lifecycle response");
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_messages_preserve_health_and_drain_through_the_binary_codec() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let local = offer(&config).unwrap();
        for operation in [operation::HEALTH, operation::DRAIN] {
            let (client_end, server_end) = std::os::unix::net::UnixStream::pair().unwrap();
            client_end
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            client_end
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            server_end
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            server_end
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let local_client = local.clone();
            let client_thread = std::thread::spawn(move || {
                let mut session = wire::Session::connect(client_end, &local_client).unwrap();
                let message = lifecycle_message(operation);
                session.send(&message).unwrap();
                let reply: wire::Control = session.receive().unwrap();
                let health = wire::Health::from_response(&reply, &message).unwrap();
                assert_eq!(health.ready, operation == operation::HEALTH);
                assert_eq!(health.draining, operation == operation::DRAIN);
            });
            let mut session = wire::Session::accept(server_end, &local).unwrap();
            let message: wire::Control = session.receive().unwrap();
            assert_eq!(message.operation, operation);
            let health = wire::Health {
                process_id: std::process::id(),
                ready: message.operation == operation::HEALTH,
                draining: message.operation == operation::DRAIN,
                ..Default::default()
            };
            session.send(&health.response(&message).unwrap()).unwrap();
            client_thread.join().unwrap();
        }
    }

    /// Both endpoints are Unix sockets in the same directory, so they share one
    /// path budget. A control name that cost more than `daemon.sock` refuses a
    /// runtime directory that has always held a daemon (kunobi-ninja/kache#1216).
    #[test]
    fn the_control_endpoint_is_no_longer_than_the_socket_it_belongs_to() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let socket = config.socket_path();
        let control = endpoint(&config);
        assert_eq!(control.parent(), socket.parent());
        assert_ne!(control, socket);
        assert!(
            control.as_os_str().len() <= socket.as_os_str().len(),
            "{} needs more path than {}",
            control.display(),
            socket.display()
        );
    }

    #[test]
    fn control_negotiation_requires_typed_health_and_advertises_drain() {
        use wire::capability;
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let local = offer(&config).unwrap();
        assert_eq!(
            local.supported,
            capability::HEALTH | capability::HEALTH_DETAILS | capability::DRAIN
        );
        assert_eq!(
            local.required,
            capability::HEALTH | capability::HEALTH_DETAILS
        );
        let mut peer = local.clone();
        peer.supported = capability::HEALTH;
        peer.required = capability::HEALTH;
        assert!(wire::negotiate(&local, &peer).is_err());
    }

    fn query(config: &Config, operation: u32) -> Result<Option<wire::Health>> {
        request(config, operation, Instant::now() + Duration::from_secs(2))
    }

    #[test]
    fn live_control_proves_startup_readiness_and_drain_without_waiting_for_admitted_work() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let lifecycle = Arc::new(Lifecycle::default());
        let server = serve(&config, Arc::clone(&lifecycle)).unwrap();
        let mut coord = DaemonCoordFile::for_socket(&config.socket_path());
        coord.control_version = Some(wire::VERSION);
        coord.write_phase(DaemonPhase::Starting).unwrap();
        let initial = query(&config, operation::HEALTH).unwrap().unwrap();
        assert!(!initial.ready);
        assert!(!initial.draining);
        assert_eq!(initial.process_id, std::process::id());
        let starter_config = config.clone();
        let starter =
            std::thread::spawn(move || lifecycle_client::ensure(&starter_config, false));
        std::thread::sleep(Duration::from_millis(100));
        assert!(!starter.is_finished(), "an initializing owner is not ready");
        assert!(
            lifecycle.accepting_calls(),
            "a compatible initializer must not be drained"
        );
        server.service.mark_ready();
        assert!(starter.join().unwrap().unwrap());
        let health = send_health_request(&config).unwrap();
        assert_eq!(health.version, VERSION);
        assert_ne!(build_epoch(), 0, "the test binary has a readable mtime");
        assert_eq!(health.build_epoch, build_epoch());
        let pending = lifecycle.begin().unwrap();
        assert!(
            query(&config, operation::HEALTH)
                .unwrap()
                .unwrap()
                .ready
        );
        let drain = query(&config, operation::DRAIN).unwrap().unwrap();
        assert!(drain.draining);
        assert!(!drain.ready);
        assert_eq!(drain.active, 1);
        assert!(lifecycle.begin().is_none());
        drop(pending);
        assert_eq!(
            query(&config, operation::HEALTH)
                .unwrap()
                .unwrap()
                .active,
            0
        );
    }

    #[test]
    fn dropping_control_server_releases_the_listener_and_handler() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let server = serve(&config, Arc::new(Lifecycle::default())).unwrap();
        let handler = Arc::downgrade(&server.service);
        std::thread::sleep(Duration::from_millis(5));
        drop(server);
        let deadline = Instant::now() + Duration::from_secs(2);
        while handler.upgrade().is_some() {
            assert!(
                Instant::now() < deadline,
                "dropped control server leaked its listener thread"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        let mut replacement = serve(&config, Arc::new(Lifecycle::default())).unwrap();
        replacement.finish();
    }

    #[test]
    fn server_exit_preserves_the_drain_acknowledgement() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let lifecycle = Arc::new(Lifecycle::default());
        let mut server = serve(&config, Arc::clone(&lifecycle)).unwrap();
        let mut coord = DaemonCoordFile::for_socket(&config.socket_path());
        coord.control_version = Some(wire::VERSION);
        coord.write_phase(DaemonPhase::Ready).unwrap();
        server.service.mark_ready();
        let probe_config = config.clone();
        let drain_config = config.clone();
        let client = std::thread::spawn(move || query(&drain_config, operation::DRAIN));
        let deadline = Instant::now() + Duration::from_secs(3);
        while !lifecycle.snapshot().draining {
            assert!(
                Instant::now() < deadline,
                "drain was never requested"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        server.finish();
        assert!(
            query(&probe_config, operation::HEALTH).is_err(),
            "finished server still accepts control requests"
        );
        drop(server);
        let acknowledged = client.join().unwrap().unwrap().unwrap();
        assert!(acknowledged.draining);
        assert!(!acknowledged.ready);
    }

    #[test]
    fn a_record_cannot_substitute_another_pid_or_cache_instance() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let lifecycle = Arc::new(Lifecycle::default());
        let _server = serve(&config, Arc::clone(&lifecycle)).unwrap();
        let mut coord = DaemonCoordFile::for_socket(&config.socket_path());
        coord.control_version = Some(wire::VERSION);
        coord.pid = std::process::id().saturating_add(1);
        coord.write_phase(DaemonPhase::Starting).unwrap();
        assert!(query(&config, operation::DRAIN).is_err());
        let failure = lifecycle_client::current(&config, Instant::now() + Duration::from_secs(2))
            .expect_err("identity mismatch must not become a retryable startup observation");
        assert!(matches!(
            failure.downcast_ref::<kunobi_daemon::client::RequestError>(),
            Some(kunobi_daemon::client::RequestError::Peer(
                kunobi_daemon::peer::Rejected::OtherProcess { .. }
            ))
        ));
        assert!(lifecycle.accepting_calls());
        let other = super::super::tests::test_config(&root.path().join("another-cache"));
        assert!(wire::negotiate(&offer(&config).unwrap(), &offer(&other).unwrap()).is_err());
        let mut foreign = offer(&config).unwrap();
        foreign.service_id[0] ^= 1;
        assert!(wire::negotiate(&offer(&config).unwrap(), &foreign).is_err());
    }

    #[test]
    fn legacy_control_distinguishes_truncation_from_oversized_responses() {
        use std::io::{BufRead, Write};
        for (payload, expected) in [
            (b"partial".to_vec(), std::io::ErrorKind::UnexpectedEof),
            (vec![b'x'; 1 << 16], std::io::ErrorKind::InvalidData),
        ] {
            let root = tempfile::tempdir().unwrap();
            let socket = root.path().join("legacy.sock");
            let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
            listener.set_nonblocking(true).unwrap();
            let server = std::thread::spawn(move || {
                for _ in 0..2 {
                    let (mut stream, _) = kunobi_daemon::readiness::wait_until(
                        Instant::now() + Duration::from_secs(5),
                        |_| match listener.accept() {
                            Ok(peer) => Ok(Some(peer)),
                            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                Ok(None)
                            }
                            Err(error) => Err(error),
                        },
                    )
                    .unwrap()
                    .expect("legacy client never connected");
                    // macOS can inherit the listener's nonblocking mode.
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut request = String::new();
                    std::io::BufReader::new(&stream)
                        .read_line(&mut request)
                        .unwrap();
                    assert!(request.ends_with('\n'));
                    stream.write_all(&payload).unwrap();
                }
            });
            let error = legacy_request(
                &socket,
                &Request::Health,
                Instant::now() + Duration::from_secs(2),
            )
            .unwrap_err();
            assert_eq!(
                error.downcast_ref::<std::io::Error>().unwrap().kind(),
                expected
            );
            let health =
                lifecycle_client::current_socket(&socket, Instant::now() + Duration::from_secs(2));
            match expected {
                std::io::ErrorKind::InvalidData => {
                    assert!(health.is_err(), "invalid frame is not retryable")
                }
                _ => assert!(
                    health.unwrap().is_none(),
                    "closed legacy peer may still be starting"
                ),
            }
            server.join().unwrap();
        }
    }

    #[test]
    fn an_unavailable_advertised_binary_endpoint_never_downgrades_to_legacy() {
        let root = tempfile::tempdir().unwrap();
        let config = super::super::tests::test_config(root.path());
        let legacy = std::os::unix::net::UnixListener::bind(config.socket_path()).unwrap();
        legacy.set_nonblocking(true).unwrap();
        let mut coord = DaemonCoordFile::for_socket(&config.socket_path());
        coord.control_version = Some(wire::VERSION);
        coord.write_phase(DaemonPhase::Ready).unwrap();
        assert!(
            lifecycle_client::current(&config, Instant::now() + Duration::from_millis(200))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            legacy.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert!(send_health_request(&config).is_err());
        assert_eq!(
            legacy.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
}
