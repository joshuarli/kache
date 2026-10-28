//! Synchronous compiler handoff with fallback rights ending at the first Commit write.

use anyhow::{Context, Result, bail, ensure};
use kache_protocol::execution::{
    EXECUTION_VERSION, ExecutionCommand, ExecutionReply, ExecutionRequest,
};
use std::io::{BufRead, Read, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::path::{Path, PathBuf};
use std::time::Duration;

const PREPARE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_REPLY_BYTES: u64 = 64 << 10;

mod unix;
mod workspace;
use unix::Connection;
mod unix_client;

/// `None` permits local execution. An error means Commit may have reached the
/// daemon and the caller must not run the compiler again.
pub fn try_execute(
    socket: &Path,
    request: &ExecutionRequest,
    descriptors: &[BorrowedFd<'_>],
) -> Result<Option<i32>> {
    let mut committed = false;
    match exchange(socket, request, descriptors, &mut committed) {
        Ok(exit) => Ok(Some(exit)),
        Err(error) if !committed => {
            tracing::debug!(%error, "daemon did not prepare compiler execution");
            Ok(None)
        }
        Err(error) => Err(error.context(
            "daemon compilation did not complete; the compiler will not be run again locally",
        )),
    }
}

fn exchange(
    socket: &Path,
    request: &ExecutionRequest,
    descriptors: &[BorrowedFd<'_>],
    committed: &mut bool,
) -> Result<i32> {
    let (mut unix_client, broker_files) = if matches!(
        request.channels,
        kache_protocol::execution::ExecutionChannels::UnixClient { .. }
    ) {
        let (client, files) = unix_client::Client::prepare(request, descriptors)?;
        (Some(client), Some(files))
    } else {
        (None, None)
    };
    let stream = Connection::connect(socket)?;
    write_message(
        &mut &stream,
        &kache_protocol::Request::ExecuteV1(Box::new(request.clone())),
    )?;
    let mut reader = std::io::BufReader::new(&stream);
    let ready = read_reply(&mut reader)?;
    {
        let ticket = match ready {
            ExecutionReply::Ready {
                version: EXECUTION_VERSION,
                descriptors: Some(ticket),
            } => ticket,
            reply => bail!("compiler execution was not ready: {reply:?}"),
        };
        let endpoint = PathBuf::from(ticket.endpoint.decode()?);
        ensure!(endpoint.is_absolute(), "invalid descriptor endpoint");
        let datagram = std::os::unix::net::UnixDatagram::unbound()?;
        datagram.set_write_timeout(Some(PREPARE_TIMEOUT))?;
        datagram.connect(endpoint)?;
        let files = {
            let mut files = descriptors.to_vec();
            if let Some(broker_files) = &broker_files {
                let kache_protocol::execution::ExecutionChannels::UnixClient { terminal, .. } =
                    request.channels
                else {
                    unreachable!()
                };
                files.splice(
                    3 + usize::from(terminal)..3 + usize::from(terminal),
                    broker_files.iter().map(AsFd::as_fd),
                );
            }
            files
        };
        let descriptors = files.as_slice();
        crate::descriptors::send(datagram.as_fd(), &ticket.key, descriptors)?;
    }

    ensure!(
        matches!(read_reply(&mut reader)?, ExecutionReply::Prepared),
        "compiler execution was not prepared"
    );

    // macOS can drain an in-flight socket when its last sender descriptor
    // closes. Prepared confirms the daemon has installed its own descriptors.
    drop(broker_files);

    stream.prepare_commit()?;
    send_commit(&mut &stream, committed).context("sending compiler Commit")?;
    ensure!(
        matches!(
            read_reply(&mut reader).context("waiting for compiler acceptance")?,
            ExecutionReply::Accepted
        ),
        "daemon did not acknowledge compiler execution"
    );
    loop {
        if let Some(client) = &mut unix_client
            && reader.buffer().is_empty()
            && client.ready(stream.as_fd())?
        {
            if !client
                .service()
                .context("servicing client-owned native operations")?
            {
                // The worker can release its broker before the async handler
                // writes Complete. Completion still comes from that handler.
                unix_client = None;
            }
            continue;
        }
        return match read_reply(&mut reader).context("waiting for compiler completion")? {
            ExecutionReply::Complete { exit_code } => Ok(exit_code),
            ExecutionReply::Failed { message } => bail!("{message}"),
            reply => bail!("invalid compiler completion: {reply:?}"),
        };
    }
}

#[cfg(test)]
fn retry_until<T>(
    deadline: Option<std::time::Instant>,
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    loop {
        match operation() {
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                ) =>
            {
                if deadline.is_some_and(|until| std::time::Instant::now() >= until) {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            result => return result,
        }
    }
}

fn send_commit(writer: &mut impl Write, committed: &mut bool) -> Result<()> {
    // Do this BEFORE the first Commit write, including partial-write errors.
    *committed = true;
    write_message(writer, &ExecutionCommand::Commit)
}

fn write_message(writer: &mut impl Write, message: &impl serde::Serialize) -> Result<()> {
    let mut frame = serde_json::to_vec(message)?;
    frame.push(b'\n');
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}

fn read_reply(reader: &mut impl BufRead) -> Result<ExecutionReply> {
    let mut frame = Vec::new();
    reader.take(MAX_REPLY_BYTES).read_until(b'\n', &mut frame)?;
    ensure!(
        frame.last() == Some(&b'\n'),
        "incomplete compiler execution response"
    );
    Ok(serde_json::from_slice(&frame)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kache_protocol::execution::{ExecutionChannels, ExecutionEntry, NativeString};
    use std::fs::File;
    use std::os::unix::net::{UnixDatagram, UnixListener};

    #[test]
    fn client_process_broker_transfers_live_pipes_and_reaps_the_native_child() {
        use kache_protocol::execution::{
            process::{Spawn, Stdio as Plan},
            unix::{Command, Reply},
        };
        use std::os::unix::net::UnixStream;
        use std::os::unix::process::ExitStatusExt;

        let root = tempfile::tempdir_in("/tmp").unwrap();
        let path = root.path().join("control");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let mut fixture = fixture(root.path());
        fixture
            .files
            .extend((0..3).map(|_| tempfile::tempfile().unwrap()));
        fixture.request.channels = ExecutionChannels::UnixClient {
            terminal: true,
            jobserver: vec![70, 71],
            trusted_pipeline: false,
        };
        std::thread::scope(|threads| {
            let server = threads.spawn(|| {
                let (mut stream, _) =
                    retry_until(Some(std::time::Instant::now() + PREPARE_TIMEOUT), || {
                        listener.accept()
                    })
                    .unwrap();
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(PREPARE_TIMEOUT)).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let endpoint = root.path().join("fds");
                let datagram = UnixDatagram::bind(&endpoint).unwrap();
                datagram.set_read_timeout(Some(PREPARE_TIMEOUT)).unwrap();
                let ticket = kache_protocol::execution::DescriptorTicket {
                    endpoint: NativeString::capture(endpoint.as_os_str()),
                    key: [31; 16],
                };
                write_message(
                    &mut stream,
                    &ExecutionReply::Ready {
                        version: EXECUTION_VERSION,
                        descriptors: Some(ticket.clone()),
                    },
                )
                .unwrap();
                // Model a receiver which has sent Ready but has not installed
                // the in-flight socket descriptors yet. macOS drains a socket
                // if its sender closes the last installed descriptor here.
                std::thread::sleep(std::time::Duration::from_millis(20));
                let mut files =
                    crate::descriptors::receive(datagram.as_fd(), &ticket.key, 8).unwrap();
                let control = UnixStream::from(files.remove(4));
                let pipes = UnixDatagram::from(files.remove(4));
                pipes.set_read_timeout(Some(PREPARE_TIMEOUT)).unwrap();
                control.set_read_timeout(Some(PREPARE_TIMEOUT)).unwrap();
                let mut control = std::io::BufReader::new(control);
                write_message(&mut stream, &ExecutionReply::Prepared).unwrap();
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert_eq!(
                    serde_json::from_str::<ExecutionCommand>(&line).unwrap(),
                    ExecutionCommand::Commit
                );
                write_message(&mut stream, &ExecutionReply::Accepted).unwrap();
                let native = |value: &str| NativeString::capture(value.as_ref());
                write_message(
                    control.get_mut(),
                    &Command::Spawn {
                        command: Box::new(Spawn {
                            program: native("/bin/sh"),
                            args: vec![
                                native("-c"),
                                native("printf '%s' \"$PPID\"; printf native-error >&2; exit 19"),
                            ],
                            env: Vec::new(),
                            cwd: NativeString::capture(root.path().as_os_str()),
                            stdio: Plan::OUTPUT,
                        }),
                        jobserver: Vec::new(),
                    },
                )
                .unwrap();
                let mut reply = || {
                    let mut line = String::new();
                    control.read_line(&mut line).unwrap();
                    serde_json::from_str::<Reply>(&line).unwrap()
                };
                let Reply::Spawned { id, pid } = reply() else {
                    panic!("missing native child");
                };
                assert_ne!(pid, std::process::id());
                let mut outputs = crate::descriptors::receive(pipes.as_fd(), &[0; 16], 2)
                    .unwrap()
                    .into_iter()
                    .map(File::from);
                let mut parent = String::new();
                outputs.next().unwrap().read_to_string(&mut parent).unwrap();
                assert_eq!(parent, std::process::id().to_string());
                let mut error = String::new();
                outputs.next().unwrap().read_to_string(&mut error).unwrap();
                assert_eq!(error, "native-error");
                let deadline = std::time::Instant::now() + PREPARE_TIMEOUT;
                let raw = loop {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "native child was never reaped"
                    );
                    write_message(control.get_mut(), &Command::TryWait { id }).unwrap();
                    let mut line = String::new();
                    control.read_line(&mut line).unwrap();
                    let Reply::Waited { status, .. } =
                        serde_json::from_str::<Reply>(&line).unwrap()
                    else {
                        panic!("missing status");
                    };
                    if let Some(status) = status {
                        break status;
                    }
                };
                assert_eq!(std::process::ExitStatus::from_raw(raw).code(), Some(19));
                write_message(control.get_mut(), &Command::Release { id }).unwrap();
                let mut line = String::new();
                control.read_line(&mut line).unwrap();
                assert_eq!(serde_json::from_str::<Reply>(&line).unwrap(), Reply::Done);
                write_message(&mut stream, &ExecutionReply::Complete { exit_code: 19 }).unwrap();
            });
            let descriptors = fixture.files.iter().map(AsFd::as_fd).collect::<Vec<_>>();
            assert_eq!(
                try_execute(&path, &fixture.request, &descriptors).unwrap(),
                Some(19)
            );
            server.join().unwrap();
        });
    }

    #[derive(Clone, Copy, Debug)]
    enum Disconnect {
        BeforeReady,
        LegacyDaemon,
        BeforePrepared,
        AfterPrepared,
        AfterCommit,
        AfterAccepted,
    }

    struct Fixture {
        request: ExecutionRequest,
        files: Vec<File>,
    }

    fn fixture(root: &Path) -> Fixture {
        let input = root.join("input");
        std::fs::write(&input, b"native-input").unwrap();
        let native = |value: &std::ffi::OsStr| NativeString::capture(value);
        Fixture {
            request: ExecutionRequest {
                version: EXECUTION_VERSION,
                entry: ExecutionEntry::Compiler,
                client_version: env!("CARGO_PKG_VERSION").into(),
                client_pid: std::process::id(),
                creation_mask: Some(0o022),
                argv: vec![native("/bin/true".as_ref())],
                env: vec![(
                    native("CLIENT".as_ref()),
                    native("execution-fixture".as_ref()),
                )],
                cwd: native(root.as_os_str()),
                home: None,
                temp: native(root.as_os_str()),
                cache: None,
                executable: native(std::env::current_exe().unwrap().as_os_str()),
                startup_micros: 0,
                channels: ExecutionChannels::Unix {
                    terminal: false,
                    jobserver: vec![],
                },
            },
            files: vec![
                File::open(input).unwrap(),
                File::create(root.join("out")).unwrap(),
                File::create(root.join("err")).unwrap(),
            ],
        }
    }

    fn disconnected_exchange(stage: Disconnect) -> Result<Option<i32>> {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        let path = root.path().join("control");
        let listener = UnixListener::bind(&path).unwrap();
        listener.set_nonblocking(true).unwrap();
        let context = fixture(root.path());
        std::thread::scope(|threads| {
            let server = threads.spawn(|| {
                let (mut stream, _) =
                    retry_until(Some(std::time::Instant::now() + PREPARE_TIMEOUT), || {
                        listener.accept()
                    })
                    .unwrap();
                stream.set_nonblocking(false).unwrap();
                stream.set_read_timeout(Some(PREPARE_TIMEOUT)).unwrap();
                stream.set_write_timeout(Some(PREPARE_TIMEOUT)).unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let kache_protocol::Request::ExecuteV1(request) =
                    serde_json::from_str(&line).unwrap()
                else {
                    panic!("wrong request: {line}");
                };
                assert_eq!(request.entry, ExecutionEntry::Compiler);
                assert_eq!(request.argv[0].decode().unwrap(), "/bin/true");
                if matches!(stage, Disconnect::BeforeReady) {
                    return;
                }
                if matches!(stage, Disconnect::LegacyDaemon) {
                    write_message(
                        &mut stream,
                        &kache_protocol::Response::err("unknown request"),
                    )
                    .unwrap();
                    return;
                }
                let endpoint = root.path().join("fds");
                let datagram = UnixDatagram::bind(&endpoint).unwrap();
                datagram.set_nonblocking(true).unwrap();
                let ticket = kache_protocol::execution::DescriptorTicket {
                    endpoint: NativeString::capture(endpoint.as_os_str()),
                    key: [29; 16],
                };
                write_message(
                    &mut stream,
                    &ExecutionReply::Ready {
                        version: EXECUTION_VERSION,
                        descriptors: Some(ticket.clone()),
                    },
                )
                .unwrap();
                let started = std::time::Instant::now();
                let files = loop {
                    match crate::descriptors::receive(datagram.as_fd(), &ticket.key, 3) {
                        Ok(files) => break files,
                        Err(error) => {
                            assert_eq!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock,
                                "receive failed: {error}"
                            );
                            assert!(started.elapsed() < PREPARE_TIMEOUT);
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    }
                };
                assert_eq!(files.len(), 3);
                if matches!(stage, Disconnect::BeforePrepared) {
                    return;
                }
                write_message(&mut stream, &ExecutionReply::Prepared).unwrap();
                if matches!(stage, Disconnect::AfterPrepared) {
                    return;
                }
                line.clear();
                reader.read_line(&mut line).unwrap();
                assert_eq!(
                    serde_json::from_str::<ExecutionCommand>(&line).unwrap(),
                    ExecutionCommand::Commit
                );
                if matches!(stage, Disconnect::AfterCommit) {
                    return;
                }
                write_message(&mut stream, &ExecutionReply::Accepted).unwrap();
            });
            let result = try_execute(
                &path,
                &context.request,
                &context.files.iter().map(AsFd::as_fd).collect::<Vec<_>>(),
            );
            server.join().unwrap();
            result
        })
    }

    #[test]
    fn execution_disconnects_before_commit_allow_local_fallback() {
        for stage in [
            Disconnect::BeforeReady,
            Disconnect::LegacyDaemon,
            Disconnect::BeforePrepared,
        ] {
            assert_eq!(disconnected_exchange(stage).unwrap(), None, "{stage:?}");
        }
    }

    #[test]
    fn execution_commit_and_completion_failures_never_allow_local_fallback() {
        for stage in [Disconnect::AfterCommit, Disconnect::AfterAccepted] {
            let error = disconnected_exchange(stage).unwrap_err();
            assert!(
                error.to_string().contains("will not be run again locally"),
                "{stage:?}: {error:#}"
            );
        }
    }

    #[test]
    fn execution_disconnect_after_prepared_cannot_report_completion() {
        // The peer can close before the timeout is cleared (safe fallback),
        // or after the first Commit write is attempted (final failure).
        match disconnected_exchange(Disconnect::AfterPrepared) {
            Ok(None) => {}
            Err(error) => assert!(error.to_string().contains("will not be run again locally")),
            Ok(Some(exit)) => panic!("disconnected peer reported completion: {exit}"),
        }
    }
}

#[cfg(test)]
mod protocol_tests {
    use super::*;

    #[test]
    fn preparation_retries_only_transient_io_and_stops_at_the_deadline() {
        let mut attempts = 0;
        let value = retry_until(
            Some(std::time::Instant::now() + Duration::from_secs(1)),
            || {
                attempts += 1;
                match attempts {
                    1 => Err(std::io::ErrorKind::Interrupted.into()),
                    2 => Err(std::io::ErrorKind::WouldBlock.into()),
                    _ => Ok(42),
                }
            },
        )
        .unwrap();
        assert_eq!(value, 42);
        assert_eq!(attempts, 3);
        for kind in [
            std::io::ErrorKind::WouldBlock,
            std::io::ErrorKind::Interrupted,
        ] {
            let error = retry_until::<()>(Some(std::time::Instant::now()), || Err(kind.into()))
                .unwrap_err();
            assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        }
        let (send, receive) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _ = send.send(retry_until::<()>(None, || {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }));
        });
        let error = receive
            .recv_timeout(PREPARE_TIMEOUT)
            .expect("non-transient errors must stop retries without a deadline")
            .unwrap_err();
        worker.join().unwrap();
        assert_eq!(error.kind(), std::io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn execution_commit_write_and_flush_failures_are_always_irrevocable() {
        struct BrokenWriter {
            remaining: usize,
        }
        impl Write for BrokenWriter {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if self.remaining == 0 {
                    return Err(std::io::ErrorKind::BrokenPipe.into());
                }
                let accepted = bytes.len().min(self.remaining);
                self.remaining -= accepted;
                Ok(accepted)
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::ErrorKind::BrokenPipe.into())
            }
        }
        for remaining in 0..=b"\"commit\"\n".len() {
            let mut committed = false;
            assert!(send_commit(&mut BrokenWriter { remaining }, &mut committed).is_err());
            assert!(
                committed,
                "write failed after {remaining} bytes without ending fallback rights"
            );
        }
    }

    #[test]
    fn execution_reply_frames_are_bounded_and_must_be_complete() {
        assert!(read_reply(&mut &b"{\"execution\":\"prepared\"}\n"[..]).is_ok());
        assert!(read_reply(&mut &b"{\"execution\":\"prepared\"}"[..]).is_err());
        assert!(read_reply(&mut &b""[..]).is_err());
        let large = vec![b'x'; usize::try_from(MAX_REPLY_BYTES).unwrap() + 10];
        let mut source = &large[..];
        assert!(read_reply(&mut source).is_err());
        assert_eq!(source.len(), 10);
    }
}
