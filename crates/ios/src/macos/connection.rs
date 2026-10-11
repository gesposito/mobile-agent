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
    /// The runner stalled, its connection alive, and this side closed the
    /// connection: why it counts as stalled.
    Stalled(String),
}

/// Why a connection to the runner closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Closed {
    /// The session ended: the runner closed it or fell silent, or this side
    /// closed it.
    Ended(String),
    /// This side closed it because the runner stalled: why it counts as
    /// stalled.
    Stalled(String),
}

impl Closed {
    /// Why, in words.
    pub(crate) fn reason(&self) -> &str {
        match self {
            Closed::Ended(reason) | Closed::Stalled(reason) => reason,
        }
    }
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
    closed: watch::Receiver<Option<Closed>>,
    negotiated: Negotiated,
    task: JoinHandle<()>,
}

impl RunnerConnection {
    /// Takes `stream`, a connection to the runner's port, and waits up to
    /// `wait` for the runner's hello, which it sends as it accepts. Calls
    /// that wait `silence` with no answer to any of them stall the runner.
    pub(crate) async fn open(
        stream: Box<dyn ReadWrite>,
        wait: Duration,
        silence: Duration,
    ) -> Result<RunnerConnection, String> {
        let (commands, receiver) = mpsc::channel(16);
        let (closed_sender, closed) = watch::channel(None);
        let (ready, hello) = oneshot::channel();
        let task = tokio::spawn(serve(stream, receiver, closed_sender, ready, silence));
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
        self.closed
            .borrow()
            .as_ref()
            .map(|closed| closed.reason().to_owned())
    }

    /// Waits until the connection closes and says why. The future owns
    /// what it reads, so a task can hold it beside others.
    pub(crate) fn closed(&self) -> impl Future<Output = Closed> + Send + 'static {
        let mut closed = self.closed.clone();
        async move {
            loop {
                let reason = closed.borrow_and_update().clone();
                if let Some(reason) = reason {
                    return reason;
                }
                if closed.changed().await.is_err() {
                    return Closed::Ended("the runner's connection ended".to_owned());
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
    closed: watch::Sender<Option<Closed>>,
    ready: oneshot::Sender<Result<Negotiated, String>>,
    silence: Duration,
) {
    let _ = silence;
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
                    closed.send_replace(Some(Closed::Ended(reason.clone())));
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

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use protocol::message::SnapshotParams;
    use protocol::session::{AgentConfig, AgentEvent, AgentSession};
    use protocol::snapshot::{Snapshot, Truncation};
    use tokio::io::DuplexStream;

    use super::*;
    use crate::macos::runner::STALL_SILENCE;

    const WAIT: Duration = Duration::from_secs(2);

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime")
    }

    /// What the scripted runner does with a request it takes.
    enum Answer {
        /// Answers it so.
        With(Result<Reply, ProtocolError>),
        /// Keeps it: no answer, not even its session's at the deadline.
        Never,
    }

    fn refusal(code: &str) -> Answer {
        Answer::With(Err(ProtocolError::new(
            code,
            format!("the scripted runner answers {code}"),
        )))
    }

    fn served() -> Answer {
        Answer::With(Ok(Reply::Snapshot(Snapshot {
            strings: Vec::new(),
            windows: Vec::new(),
            node_count: 0,
            truncated: Truncation::EMPTY,
            seq: 0,
        })))
    }

    fn snapshot() -> Call {
        Call::Snapshot(SnapshotParams::default())
    }

    /// The runner's end of the connection: the protocol's `AgentSession` on
    /// `stream`, answering the requests it takes with `answers` in turn and
    /// keeping those past them, and answering every ping; its session
    /// answers no passed deadline.
    async fn runner(mut stream: DuplexStream, answers: Vec<Answer>) {
        let config = AgentConfig {
            agent: "placeholder-runner 0.0.0".into(),
            capabilities: Capabilities::TREE,
            ..AgentConfig::default()
        };
        let mut session = AgentSession::new(config, Instant::now());
        let mut answers: VecDeque<Answer> = answers.into();
        let mut buffer = vec![0u8; 64 * 1024];
        loop {
            while let Some(event) = session.poll_event() {
                match event {
                    AgentEvent::Request(request) => {
                        if let Some(Answer::With(result)) = answers.pop_front() {
                            session.respond(request.id, result);
                        }
                    }
                    AgentEvent::Closed(_) => return,
                    AgentEvent::Ready(_) => {}
                }
            }
            while let Some(bytes) = session.poll_transmit() {
                if stream.write_all(&bytes).await.is_err() {
                    return;
                }
            }
            match stream.read(&mut buffer).await {
                Ok(0) | Err(_) => return,
                Ok(read) => session.receive(&buffer[..read], Instant::now()),
            }
        }
    }

    /// A connection to the scripted runner, its calls stalling the runner
    /// once they waited `silence` with no answer.
    async fn connect(answers: Vec<Answer>, silence: Duration) -> RunnerConnection {
        let (client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(runner(server, answers));
        RunnerConnection::open(Box::new(client), WAIT, silence)
            .await
            .expect("the runner's hello")
    }

    /// Physical-ios, task 9.7 (design D26): two calls in a row answered
    /// `deadline_exceeded` or `busy` with the connection alive, as the
    /// phone's runner answered the call after the text set its worker never
    /// came back from (`probes/text-hang-swiftui-*.jsonl`), stall the
    /// runner: the second call fails as stalled, naming both answers, and
    /// the connection closes as stalled, so that the supervisor ends the
    /// runner and starts it again; no later call is sent.
    #[test]
    fn two_calls_in_a_row_answered_deadline_exceeded_or_busy_stall_the_runner() {
        runtime().block_on(async {
            let connection = connect(
                vec![refusal(code::DEADLINE_EXCEEDED), refusal(code::BUSY)],
                STALL_SILENCE,
            )
            .await;
            let first = connection.call(snapshot(), WAIT).await;
            assert!(
                matches!(&first, Err(CallFailure::Refused(error)) if error.code == code::DEADLINE_EXCEEDED),
                "one refusal alone is the caller's to repeat: {first:?}"
            );
            let second = connection.call(snapshot(), WAIT).await;
            let Err(CallFailure::Stalled(why)) = second else {
                panic!("the second refusal stalls the runner: {second:?}");
            };
            assert!(
                why.contains(code::DEADLINE_EXCEEDED) && why.contains(code::BUSY),
                "{why}"
            );
            let closed = timeout(WAIT, connection.closed()).await.expect("closed");
            assert_eq!(closed, Closed::Stalled(why));
            let after = connection.call(snapshot(), WAIT).await;
            assert!(matches!(after, Err(CallFailure::NotSent(_))), "{after:?}");
        });
    }

    /// Physical-ios, task 9.7: an answer between two refusals, a reply or
    /// a refusal of another code, which the runner's worker gave, keeps the
    /// runner serving, and so does a refusal alone.
    #[test]
    fn an_answer_between_two_refusals_keeps_the_runner_serving() {
        runtime().block_on(async {
            let codes = [
                Some(code::BUSY),
                None,
                Some(code::DEADLINE_EXCEEDED),
                Some(code::WINDOW_CHANGED),
                Some(code::BUSY),
                None,
            ];
            let answers = codes
                .iter()
                .map(|code| code.map_or_else(served, refusal))
                .collect();
            let connection = connect(answers, STALL_SILENCE).await;
            for expected in codes {
                let answered = connection.call(snapshot(), WAIT).await;
                match (expected, &answered) {
                    (None, Ok(_)) => {}
                    (Some(code), Err(CallFailure::Refused(error))) if error.code == code => {}
                    _ => panic!("{expected:?} expected: {answered:?}"),
                }
            }
            assert_eq!(connection.closed_reason(), None, "the connection serves");
        });
    }

    /// Physical-ios, task 9.7: a call the runner leaves unanswered while it
    /// answers pings, the connection alive, stalls the runner once calls
    /// waited the silence bound with no answer to any of them, before the
    /// call's own deadline: the call fails as stalled and the connection
    /// closes so. The host pings after a second without hearing anything,
    /// so the runner answered a ping before the bound.
    #[test]
    fn a_call_left_unanswered_past_the_silence_bound_stalls_the_runner_though_it_answers_pings() {
        runtime().block_on(async {
            let silence = Duration::from_millis(1500);
            let connection = connect(vec![Answer::Never], silence).await;
            let started = Instant::now();
            let answered = connection.call(snapshot(), Duration::from_secs(10)).await;
            let waited = started.elapsed();
            let Err(CallFailure::Stalled(why)) = answered else {
                panic!("stalled at the silence bound: {answered:?}");
            };
            assert!(why.contains("1.5s"), "{why}");
            assert!(
                waited >= silence && waited < silence + Duration::from_secs(1),
                "{waited:?}"
            );
            let closed = timeout(WAIT, connection.closed()).await.expect("closed");
            assert!(matches!(closed, Closed::Stalled(_)), "{closed:?}");
        });
    }
}
