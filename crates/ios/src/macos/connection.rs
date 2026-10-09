//! One connection to the runner (design D20): the protocol crate's
//! `HostSession` driven over the runner's socket, a usbmuxd `Connect` to
//! its loopback port, by a task that owns the socket. The task feeds the
//! session what arrives, writes what it queues, acts on its deadlines and
//! liveness (pings every second, the runner declared gone after three
//! silent ones), and hands each response to the call waiting for it.

use std::collections::HashMap;
use std::future::Future;
use std::time::{Duration, Instant};

use idevice::ReadWrite;
use protocol::code;
use protocol::message::{Call, Reply};
use protocol::session::{HostConfig, HostEvent, HostSession, Negotiated};
use protocol::{Capabilities, Error as ProtocolError};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::timeout;

/// Why a call got no reply from the runner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CallFailure {
    /// The connection was closed before the request was written, so the
    /// runner never saw it.
    NotSent(String),
    /// The connection closed after the request was written and before its
    /// answer: the runner may have acted on it.
    Lost(String),
    /// The runner, or the session on its behalf, answered with an error.
    Refused(ProtocolError),
}

enum Command {
    Call {
        call: Call,
        deadline: Duration,
        reply: oneshot::Sender<Result<Reply, CallFailure>>,
    },
    Close,
}

/// A live connection to the runner. Dropping it closes the connection.
pub(crate) struct RunnerConnection {
    commands: mpsc::Sender<Command>,
    closed: watch::Receiver<Option<String>>,
    negotiated: Negotiated,
    task: JoinHandle<()>,
}

impl RunnerConnection {
    /// Takes `stream`, a connection to the runner's port, and waits up to
    /// `wait` for the runner's hello, which it sends as it accepts.
    pub(crate) async fn open(
        stream: Box<dyn ReadWrite>,
        wait: Duration,
    ) -> Result<RunnerConnection, String> {
        let (commands, receiver) = mpsc::channel(16);
        let (closed_sender, closed) = watch::channel(None);
        let (ready, hello) = oneshot::channel();
        let task = tokio::spawn(serve(stream, receiver, closed_sender, ready));
        match timeout(wait, hello).await {
            Ok(Ok(Ok(negotiated))) => Ok(RunnerConnection {
                commands,
                closed,
                negotiated,
                task,
            }),
            Ok(Ok(Err(reason))) => Err(reason),
            Ok(Err(_)) => Err("the connection's task ended before the hello".into()),
            Err(_) => {
                task.abort();
                Err(format!("no hello within {wait:?}"))
            }
        }
    }

    /// The agent the hello named: the runner's name and version.
    pub(crate) fn agent(&self) -> &str {
        &self.negotiated.agent
    }

    /// What the hello advertised.
    pub(crate) fn capabilities(&self) -> Capabilities {
        self.negotiated.capabilities
    }

    /// Sends `call` with `deadline` as its deadline and waits for the
    /// answer; the session itself gives up half a second past the deadline.
    pub(crate) async fn call(&self, call: Call, deadline: Duration) -> Result<Reply, CallFailure> {
        let (reply, answer) = oneshot::channel();
        let command = Command::Call {
            call,
            deadline,
            reply,
        };
        if self.commands.send(command).await.is_err() {
            return Err(CallFailure::NotSent(
                self.closed_reason()
                    .unwrap_or_else(|| "the runner's connection is closed".to_owned()),
            ));
        }
        answer.await.unwrap_or_else(|_| {
            Err(CallFailure::Lost(
                "the runner's connection ended".to_owned(),
            ))
        })
    }

    /// Why the connection closed, once it has.
    pub(crate) fn closed_reason(&self) -> Option<String> {
        self.closed.borrow().clone()
    }

    /// Waits until the connection closes and says why. The future owns
    /// what it reads, so a task can hold it beside others.
    pub(crate) fn closed(&self) -> impl Future<Output = String> + Send + 'static {
        let mut closed = self.closed.clone();
        async move {
            loop {
                let reason = closed.borrow_and_update().clone();
                if let Some(reason) = reason {
                    return reason;
                }
                if closed.changed().await.is_err() {
                    return "the runner's connection ended".to_owned();
                }
            }
        }
    }

    /// Closes the connection from this side; calls waiting fail.
    pub(crate) async fn close(&self) {
        let _ = self.commands.send(Command::Close).await;
    }
}

impl Drop for RunnerConnection {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The failure a session error stands for: the session reports a lost
/// connection as `disconnected`.
fn failure(error: ProtocolError, sent: bool) -> CallFailure {
    if error.code == code::DISCONNECTED {
        if sent {
            CallFailure::Lost(error.message)
        } else {
            CallFailure::NotSent(error.message)
        }
    } else {
        CallFailure::Refused(error)
    }
}

/// The connection's task.
async fn serve(
    mut stream: Box<dyn ReadWrite>,
    mut commands: mpsc::Receiver<Command>,
    closed: watch::Sender<Option<String>>,
    ready: oneshot::Sender<Result<Negotiated, String>>,
) {
    let mut session = HostSession::new(HostConfig::default(), Instant::now());
    let mut pending: HashMap<u32, oneshot::Sender<Result<Reply, CallFailure>>> = HashMap::new();
    let mut ready = Some(ready);
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        while let Some(bytes) = session.poll_transmit() {
            if stream.write_all(&bytes).await.is_err() {
                session.receive_eof(Instant::now());
            }
        }
        let _ = stream.flush().await;
        while let Some(event) = session.poll_event() {
            match event {
                HostEvent::Ready(negotiated) => {
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(Ok(negotiated));
                    }
                }
                HostEvent::Response { id, result, .. } => {
                    if let Some(reply) = pending.remove(&id) {
                        let _ = reply.send(result.map_err(|error| failure(error, true)));
                    }
                }
                HostEvent::Closed(reason) => {
                    let reason = reason.to_string();
                    if let Some(ready) = ready.take() {
                        let _ = ready.send(Err(reason.clone()));
                    }
                    for (_, reply) in pending.drain() {
                        let _ = reply.send(Err(CallFailure::Lost(reason.clone())));
                    }
                    closed.send_replace(Some(reason.clone()));
                    commands.close();
                    while let Ok(command) = commands.try_recv() {
                        if let Command::Call { reply, .. } = command {
                            let _ = reply.send(Err(CallFailure::NotSent(reason.clone())));
                        }
                    }
                    return;
                }
                // The runner sends no events; a gap or a resync of a stream
                // it does not use changes nothing here.
                _ => {}
            }
        }
        let wake = session
            .poll_timeout()
            .map(tokio::time::Instant::from_std)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            read = stream.read(&mut buffer) => match read {
                Ok(0) | Err(_) => session.receive_eof(Instant::now()),
                Ok(read) => session.receive(&buffer[..read], Instant::now()),
            },
            command = commands.recv() => match command {
                Some(Command::Call { call, deadline, reply }) => {
                    match session.send_request(call, Some(deadline), Instant::now()) {
                        Ok(id) => {
                            pending.insert(id, reply);
                        }
                        Err(error) => {
                            let _ = reply.send(Err(failure(error, false)));
                        }
                    }
                }
                Some(Command::Close) | None => session.close(),
            },
            () = tokio::time::sleep_until(wake) => session.handle_timeout(Instant::now()),
        }
    }
}
