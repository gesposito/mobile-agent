//! DVT's process control and device information (design D19), the calls
//! written by hand: `idevice` 0.1.68's `kill_app` sends the pid as a
//! primitive integer, which on iOS 26.6.2 left the app running and dropped
//! every DVT connection (spike S9), and its `launch_app` drops the device's
//! reason for a refusal. Each channel lives on a DVT connection of its own,
//! served by a task that owns the connection, so the channel stays open for
//! the target's lifetime and its calls go in one at a time.

use std::time::Duration;

use idevice::ReadWrite;
use idevice::dvt::message::{AuxValue, Message};
use idevice::dvt::remote_server::RemoteServerClient;
use plist::{Dictionary, Value};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use super::host::describe;
use crate::model::LaunchOptions;

/// DVT's process control: launches and signals.
pub(crate) const PROCESS_CONTROL: &str = "com.apple.instruments.server.services.processcontrol";
/// DVT's device information: the process list.
pub(crate) const DEVICE_INFO: &str = "com.apple.instruments.server.services.deviceinfo";

pub(crate) const LAUNCH: &str =
    "launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:";
pub(crate) const SEND_SIGNAL: &str = "sendSignal:toPid:";
pub(crate) const RUNNING_PROCESSES: &str = "runningProcesses";
pub(crate) const IS_RUNNING_PID: &str = "isRunningPid:";
pub(crate) const SIGKILL: i64 = 9;

/// One method call on a channel: its selector and arguments.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Call {
    pub selector: &'static str,
    pub args: Vec<AuxValue>,
}

/// The launch of `bundle_id` with `options`: no device path, the
/// environment and arguments archived, the app started running rather than
/// suspended, and `KillExisting` as `terminate_running` asks.
pub(crate) fn launch(bundle_id: &str, options: &LaunchOptions) -> Call {
    let environment: Dictionary = options
        .environment
        .iter()
        .map(|(name, value)| (name.clone(), Value::String(value.clone())))
        .collect();
    let arguments = options
        .arguments
        .iter()
        .map(|argument| Value::String(argument.clone()))
        .collect();
    let mut launch_options = Dictionary::new();
    launch_options.insert("StartSuspendedKey".into(), Value::Boolean(false));
    launch_options.insert(
        "KillExisting".into(),
        Value::Boolean(options.terminate_running),
    );
    Call {
        selector: LAUNCH,
        args: vec![
            AuxValue::archived_value(Value::String(String::new())),
            AuxValue::archived_value(Value::String(bundle_id.to_owned())),
            AuxValue::archived_value(Value::Dictionary(environment)),
            AuxValue::archived_value(Value::Array(arguments)),
            AuxValue::archived_value(Value::Dictionary(launch_options)),
        ],
    }
}

/// `sendSignal:toPid:` with `signal` and `pid` both archived as NSNumbers;
/// its reply confirms the signal (S9: 2.9 ms at p50, the process gone from
/// the list 22 ms after the request).
pub(crate) fn send_signal(pid: u32, signal: i64) -> Call {
    Call {
        selector: SEND_SIGNAL,
        args: vec![
            AuxValue::archived_value(Value::Integer(signal.into())),
            AuxValue::archived_value(Value::Integer(i64::from(pid).into())),
        ],
    }
}

/// The process list.
pub(crate) fn running_processes() -> Call {
    Call {
        selector: RUNNING_PROCESSES,
        args: Vec::new(),
    }
}

/// Whether the process `pid` runs: a small reply, where the whole list
/// takes 480 to 710 ms on the iPhone SE (3rd generation) with about 580
/// processes.
pub(crate) fn is_running_pid(pid: u32) -> Call {
    Call {
        selector: IS_RUNNING_PID,
        args: vec![AuxValue::archived_value(Value::Integer(
            i64::from(pid).into(),
        ))],
    }
}

/// What an `isRunningPid:` reply says.
pub(crate) fn running_reply(reply: &Message) -> Result<bool, String> {
    match &reply.data {
        Some(Value::Boolean(running)) => Ok(*running),
        other => Err(format!("the reply carried {other:?}, not a yes or no")),
    }
}

/// A process in DVT's list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Process {
    pub pid: u32,
    pub name: String,
    /// `bundleIdentifier`, which iOS 26.6.2 gives application processes;
    /// an app's extensions have identifiers of their own.
    pub bundle_id: Option<String>,
    pub is_application: bool,
}

/// The pid a launch's reply carries, or why the device refused.
pub(crate) fn launched_pid(reply: &Message) -> Result<u32, String> {
    if let Some(Value::Integer(pid)) = &reply.data {
        return pid
            .as_unsigned()
            .and_then(|pid| u32::try_from(pid).ok())
            .ok_or_else(|| format!("the reply's pid, {pid:?}, is not a pid"));
    }
    Err(reply
        .raw_data
        .as_deref()
        .and_then(ns_error)
        .unwrap_or_else(|| format!("the reply carried {:?}, not a pid", reply.data)))
}

/// The processes a `runningProcesses` reply lists.
pub(crate) fn processes(reply: &Message) -> Result<Vec<Process>, String> {
    let entries = reply
        .data
        .as_ref()
        .and_then(Value::as_array)
        .ok_or_else(|| format!("the reply carried {:?}, not a list", reply.data))?;
    Ok(entries
        .iter()
        .filter_map(Value::as_dictionary)
        .filter_map(|entry| {
            let pid = entry
                .get("pid")
                .and_then(Value::as_unsigned_integer)
                .and_then(|pid| u32::try_from(pid).ok())?;
            let text = |key: &str| entry.get(key).and_then(Value::as_string).map(str::to_owned);
            Some(Process {
                pid,
                name: text("name").unwrap_or_default(),
                bundle_id: text("bundleIdentifier"),
                is_application: entry
                    .get("isApplication")
                    .and_then(Value::as_boolean)
                    .unwrap_or(false),
            })
        })
        .collect())
}

/// An archived `NSError`'s domain, code and description, from the archive's
/// bytes: `idevice`'s decoder leaves its fields as unresolved references.
pub(crate) fn ns_error(archive: &[u8]) -> Option<String> {
    let archive: Dictionary = plist::from_bytes(archive).ok()?;
    let objects = archive.get("$objects")?.as_array()?;
    let root = archive
        .get("$top")?
        .as_dictionary()?
        .get("root")
        .map(|root| resolve(objects, root, 0))?;
    let error = root.as_dictionary()?;
    let domain = error.get("NSDomain")?.as_string()?;
    let code = error.get("NSCode")?.as_signed_integer()?;
    let info = error.get("NSUserInfo").and_then(Value::as_dictionary);
    let detail = [
        "NSLocalizedDescription",
        "NSDebugDescription",
        "NSLocalizedFailureReason",
    ]
    .iter()
    .find_map(|key| info?.get(key)?.as_string());
    Some(match detail {
        Some(detail) => format!("{domain} {code}: {detail}"),
        None => format!("{domain} {code}"),
    })
}

/// `value` with an archive's references followed, `NSDictionary` and
/// `NSArray` shapes read as dictionaries and arrays, `$null` as nothing,
/// and other objects as their fields, at most 16 levels deep.
fn resolve(objects: &[Value], value: &Value, depth: usize) -> Value {
    if depth > 16 {
        return Value::String(String::new());
    }
    match value {
        Value::Uid(uid) => match usize::try_from(uid.get())
            .ok()
            .and_then(|index| objects.get(index))
        {
            Some(object) => resolve(objects, object, depth + 1),
            None => Value::String(String::new()),
        },
        Value::String(text) if text == "$null" => Value::String(String::new()),
        Value::Dictionary(fields) => {
            let keys = fields.get("NS.keys").and_then(Value::as_array);
            let values = fields.get("NS.objects").and_then(Value::as_array);
            match (keys, values) {
                (Some(keys), Some(values)) => Value::Dictionary(
                    keys.iter()
                        .zip(values)
                        .filter_map(|(key, value)| {
                            let key = resolve(objects, key, depth + 1);
                            Some((
                                key.as_string()?.to_owned(),
                                resolve(objects, value, depth + 1),
                            ))
                        })
                        .collect(),
                ),
                (None, Some(values)) => Value::Array(
                    values
                        .iter()
                        .map(|value| resolve(objects, value, depth + 1))
                        .collect(),
                ),
                _ => match fields.get("NS.string") {
                    Some(text) => resolve(objects, text, depth + 1),
                    None => Value::Dictionary(
                        fields
                            .iter()
                            .filter(|(key, _)| *key != "$class")
                            .map(|(key, value)| (key.clone(), resolve(objects, value, depth + 1)))
                            .collect(),
                    ),
                },
            }
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| resolve(objects, item, depth + 1))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Why a call failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CallError {
    /// The connection was gone before the call was written, so a new one
    /// may serve it.
    NotSent(String),
    /// The connection closed after the call was written, before its reply.
    Closed(String),
    /// No reply came within the wait; the channel is ended.
    TimedOut(Duration),
}

/// A call waiting for the channel's task.
struct Request {
    call: Call,
    reply: oneshot::Sender<Result<Message, CallError>>,
}

/// One DVT channel on a connection of its own.
pub(crate) struct DvtChannel {
    identifier: &'static str,
    requests: mpsc::Sender<Request>,
    task: JoinHandle<()>,
}

impl DvtChannel {
    /// Opens the channel `identifier` on `client`, a DVT connection whose
    /// capabilities were read, and serves it on a task of its own.
    pub(crate) async fn open(
        client: RemoteServerClient<Box<dyn ReadWrite>>,
        identifier: &'static str,
        wait: Duration,
    ) -> Result<DvtChannel, String> {
        let (requests, waiting) = mpsc::channel(8);
        let (opened, ready) = oneshot::channel();
        let task = tokio::spawn(serve(client, identifier, opened, waiting));
        match timeout(wait, ready).await {
            Ok(Ok(Ok(()))) => Ok(DvtChannel {
                identifier,
                requests,
                task,
            }),
            Ok(Ok(Err(reason))) => Err(reason),
            Ok(Err(_)) => Err(format!("{identifier}'s task ended before it opened")),
            Err(_) => {
                task.abort();
                Err(format!("DVT did not open {identifier} within {wait:?}"))
            }
        }
    }

    /// Sends `call` and waits up to `wait` for its reply. A reply that does
    /// not come ends the channel, so that it cannot answer a later call.
    pub(crate) async fn call(&self, call: Call, wait: Duration) -> Result<Message, CallError> {
        let (reply, answer) = oneshot::channel();
        if self.requests.send(Request { call, reply }).await.is_err() {
            return Err(CallError::NotSent(format!(
                "{}'s connection is closed",
                self.identifier
            )));
        }
        match timeout(wait, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(CallError::Closed(format!(
                "{}'s task ended",
                self.identifier
            ))),
            Err(_) => {
                self.task.abort();
                Err(CallError::TimedOut(wait))
            }
        }
    }

    /// Whether the channel's connection still serves.
    pub(crate) fn alive(&self) -> bool {
        !self.task.is_finished() && !self.requests.is_closed()
    }
}

impl Drop for DvtChannel {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The channel's task: opens the channel on the connection it owns, says
/// so, then sends each call and waits for its reply, in order. A call that
/// cannot be written, or a reply that cannot be read, ends it with the
/// connection.
async fn serve(
    mut client: RemoteServerClient<Box<dyn ReadWrite>>,
    identifier: &'static str,
    opened: oneshot::Sender<Result<(), String>>,
    mut requests: mpsc::Receiver<Request>,
) {
    let mut channel = match client.make_channel(identifier).await {
        Ok(channel) => channel,
        Err(error) => {
            let _ = opened.send(Err(format!(
                "DVT did not open {identifier}: {}",
                describe(&error)
            )));
            return;
        }
    };
    if opened.send(Ok(())).is_err() {
        return;
    }
    while let Some(Request { call, reply }) = requests.recv().await {
        let args = (!call.args.is_empty()).then_some(call.args);
        let sent = channel
            .call_method(Some(Value::String(call.selector.into())), args, true)
            .await;
        if let Err(error) = sent {
            let _ = reply.send(Err(CallError::NotSent(describe(&error))));
            return;
        }
        match channel.read_message().await {
            Ok(message) => {
                let _ = reply.send(Ok(message));
            }
            Err(error) => {
                let _ = reply.send(Err(CallError::Closed(describe(&error))));
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::macos::fake::{Arg, FakeDevice, ns_error_archive, serve_dvt};

    const WAIT: Duration = Duration::from_millis(500);
    const SWIFTUI: &str = "dev.mobileagent.fixture.swiftui";

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("a runtime")
    }

    /// A channel to the fake device's DVT over a connection of its own, its
    /// capabilities read first as the driver reads them.
    async fn channel(device: &Arc<FakeDevice>, identifier: &'static str) -> DvtChannel {
        let (client, server) = tokio::io::duplex(1 << 20);
        tokio::spawn(serve_dvt(Arc::clone(device), server));
        let mut client = RemoteServerClient::new(client);
        client.read_message(0).await.expect("DVT's capabilities");
        DvtChannel::open(client, identifier, WAIT)
            .await
            .expect("the channel opens")
    }

    #[test]
    fn a_launch_sends_the_bundle_its_environment_arguments_and_kill_existing_and_reads_the_pid() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            device.install_app(SWIFTUI, "FixtureSwiftUI");
            let control = channel(&device, PROCESS_CONTROL).await;
            let options = LaunchOptions::new()
                .argument("-FixtureFlag")
                .env("FIXTURE_INITIAL_TAB", "Big")
                .terminating_running();
            let reply = control
                .call(launch(SWIFTUI, &options), WAIT)
                .await
                .expect("a reply");
            let pid = launched_pid(&reply).expect("a pid");
            assert!(device.running(SWIFTUI).contains(&pid));
            let calls = device.dvt_calls();
            let call = calls.last().expect("the launch");
            assert_eq!(call.service, PROCESS_CONTROL);
            assert_eq!(call.selector, LAUNCH);
            let mut environment = Dictionary::new();
            environment.insert("FIXTURE_INITIAL_TAB".into(), Value::String("Big".into()));
            let mut launch_options = Dictionary::new();
            launch_options.insert("StartSuspendedKey".into(), Value::Boolean(false));
            launch_options.insert("KillExisting".into(), Value::Boolean(true));
            assert_eq!(
                call.args,
                vec![
                    Arg::Archived(Value::String(String::new())),
                    Arg::Archived(Value::String(SWIFTUI.into())),
                    Arg::Archived(Value::Dictionary(environment)),
                    Arg::Archived(Value::Array(vec![Value::String("-FixtureFlag".into())])),
                    Arg::Archived(Value::Dictionary(launch_options)),
                ]
            );
        });
    }

    #[test]
    fn a_launch_that_keeps_a_running_instance_leaves_kill_existing_off() {
        let call = launch(SWIFTUI, &LaunchOptions::new());
        let options = Dictionary::from_iter([
            ("StartSuspendedKey".to_owned(), Value::Boolean(false)),
            ("KillExisting".to_owned(), Value::Boolean(false)),
        ]);
        assert_eq!(
            call.args[4],
            AuxValue::archived_value(Value::Dictionary(options))
        );
        assert_eq!(
            call.args[2],
            AuxValue::archived_value(Value::Dictionary(Dictionary::new()))
        );
        assert_eq!(
            call.args[3],
            AuxValue::archived_value(Value::Array(Vec::new()))
        );
    }

    #[test]
    fn a_kill_is_send_signal_to_pid_with_sigkill_and_the_pid_both_archived() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            device.install_app(SWIFTUI, "FixtureSwiftUI");
            let control = channel(&device, PROCESS_CONTROL).await;
            let reply = control
                .call(launch(SWIFTUI, &LaunchOptions::new()), WAIT)
                .await
                .expect("launched");
            let pid = launched_pid(&reply).expect("a pid");
            control
                .call(send_signal(pid, SIGKILL), WAIT)
                .await
                .expect("the signal's reply");
            assert!(device.running(SWIFTUI).is_empty(), "the process is gone");
            let calls = device.dvt_calls();
            let kill = calls.last().expect("the signal");
            assert_eq!(kill.selector, SEND_SIGNAL);
            // idevice 0.1.68's kill_app sends the pid as a primitive, which
            // stalls the phone's DVT service (S9); both go archived.
            assert_eq!(
                kill.args,
                vec![
                    Arg::Archived(Value::Integer(9.into())),
                    Arg::Archived(Value::Integer(i64::from(pid).into())),
                ]
            );
        });
    }

    #[test]
    fn the_process_list_gives_each_application_its_bundle_identifier() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            device.install_app(SWIFTUI, "FixtureSwiftUI");
            let control = channel(&device, PROCESS_CONTROL).await;
            let reply = control
                .call(launch(SWIFTUI, &LaunchOptions::new()), WAIT)
                .await
                .expect("launched");
            let pid = launched_pid(&reply).expect("a pid");
            let info = channel(&device, DEVICE_INFO).await;
            let reply = info
                .call(running_processes(), WAIT)
                .await
                .expect("the list");
            let listed = processes(&reply).expect("processes");
            assert!(listed.contains(&Process {
                pid,
                name: "FixtureSwiftUI".into(),
                bundle_id: Some(SWIFTUI.into()),
                is_application: true,
            }));
            assert!(
                listed
                    .iter()
                    .any(|process| process.bundle_id.is_none() && !process.is_application),
                "daemons have no bundle identifier"
            );
            assert_eq!(
                device.dvt_calls().last().expect("a call").service,
                DEVICE_INFO
            );
        });
    }

    #[test]
    fn whether_a_pid_runs_is_asked_with_the_pid_archived() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            device.install_app(SWIFTUI, "FixtureSwiftUI");
            let control = channel(&device, PROCESS_CONTROL).await;
            let reply = control
                .call(launch(SWIFTUI, &LaunchOptions::new()), WAIT)
                .await
                .expect("launched");
            let pid = launched_pid(&reply).expect("a pid");
            let info = channel(&device, DEVICE_INFO).await;
            let reply = info.call(is_running_pid(pid), WAIT).await.expect("a reply");
            assert_eq!(running_reply(&reply), Ok(true));
            let call = device.dvt_calls().last().expect("the question").clone();
            assert_eq!(call.selector, IS_RUNNING_PID);
            assert_eq!(
                call.args,
                vec![Arg::Archived(Value::Integer(i64::from(pid).into()))]
            );
            let reply = info
                .call(is_running_pid(pid + 1), WAIT)
                .await
                .expect("a reply");
            assert_eq!(running_reply(&reply), Ok(false));
        });
    }

    #[test]
    fn a_refused_launch_reads_the_errors_domain_code_and_description() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let control = channel(&device, PROCESS_CONTROL).await;
            let reply = control
                .call(
                    launch("dev.mobileagent.placeholder.missing", &LaunchOptions::new()),
                    WAIT,
                )
                .await
                .expect("a refusal is a reply");
            let reason = launched_pid(&reply).expect_err("no pid");
            for part in [
                "FBSOpenApplicationServiceErrorDomain",
                "4",
                "dev.mobileagent.placeholder.missing",
            ] {
                assert!(reason.contains(part), "{part:?} is missing from: {reason}");
            }
        });
    }

    #[test]
    fn an_archived_error_reads_as_its_domain_code_and_description() {
        let archive = ns_error_archive("NSPOSIXErrorDomain", 3, "No such process");
        assert_eq!(
            ns_error(&archive).as_deref(),
            Some("NSPOSIXErrorDomain 3: No such process")
        );
        assert_eq!(ns_error(b"not an archive"), None);
    }

    #[test]
    fn a_call_on_a_connection_that_closed_fails_and_the_channel_is_done() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let info = channel(&device, DEVICE_INFO).await;
            assert!(info.alive());
            device.close_dvt();
            let error = info
                .call(running_processes(), WAIT)
                .await
                .expect_err("the connection is closed");
            assert!(
                matches!(error, CallError::NotSent(_) | CallError::Closed(_)),
                "{error:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(!info.alive());
        });
    }

    #[test]
    fn a_call_without_a_reply_times_out_and_ends_the_channel() {
        runtime().block_on(async {
            let device = FakeDevice::recorded();
            let info = channel(&device, DEVICE_INFO).await;
            device.silence_dvt();
            let error = info
                .call(running_processes(), Duration::from_millis(100))
                .await
                .expect_err("no reply");
            assert_eq!(error, CallError::TimedOut(Duration::from_millis(100)));
            tokio::time::sleep(Duration::from_millis(50)).await;
            assert!(!info.alive(), "a late reply must not answer the next call");
        });
    }
}
