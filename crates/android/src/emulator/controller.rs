//! The controller client: two HTTP/2 connections to one emulator, a deadline on
//! every call, touches, raw screenshots and the scaled frame stream.

use std::fmt;
use std::future::Future;
use std::time::{Duration, Instant};

use tonic::metadata::{Ascii, MetadataValue};
use tonic::service::Interceptor;
use tonic::service::interceptor::InterceptedService;
use tonic::transport::Channel;

use super::endpoint::{Endpoint, Unavailable};
use super::frame::{Frame, FrameError};
use super::hardware::{Hardware, scaled_size};
use super::proto;
use super::proto::emulator_controller_client::EmulatorControllerClient;
use super::proto::image_format::ImgFormat;

/// Largest message the client accepts: a native RGBA8888 frame of a 1080x2400
/// display is 10.4 MB, beyond tonic's default of 4 MB.
pub const MAX_MESSAGE: usize = 64 << 20;

/// Pressure of a finger on the glass; the touchscreen's axis maximum is 1024.
pub const DOWN_PRESSURE: i32 = 512;

/// Deadlines of the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Deadline of every unary call, and of opening the frame stream.
    pub deadline: Duration,
    /// Deadline of each of the two connections.
    pub connect_deadline: Duration,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            deadline: Duration::from_secs(2),
            connect_deadline: Duration::from_secs(2),
        }
    }
}

/// Why a call on a connected controller failed.
#[derive(Debug)]
pub enum Error {
    /// The call did not finish within its deadline.
    Deadline { call: &'static str, after: Duration },
    /// The controller answered with an error status.
    Status {
        call: &'static str,
        status: tonic::Status,
    },
    /// A frame's bytes do not match its dimensions. The stream stays usable.
    Frame {
        call: &'static str,
        error: FrameError,
    },
    /// The emulator ended the frame stream.
    StreamEnded,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Deadline { call, after } => write!(f, "{call}: no answer within {after:?}"),
            Error::Status { call, status } => {
                write!(f, "{call}: {:?}: {}", status.code(), status.message())
            }
            Error::Frame { call, error } => write!(f, "{call}: {error}"),
            Error::StreamEnded => write!(f, "the emulator ended the frame stream"),
        }
    }
}

impl std::error::Error for Error {}

/// One contact in a touch event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TouchPoint {
    /// Tracks a contact across events; any non-negative value.
    pub id: i32,
    /// Display pixels from the left edge.
    pub x: i32,
    /// Display pixels from the top edge.
    pub y: i32,
    /// Non-zero while touching; zero lifts the contact.
    pub pressure: i32,
}

impl TouchPoint {
    /// Contact 0 pressed at (x, y).
    pub fn down(x: i32, y: i32) -> Self {
        Self {
            id: 0,
            x,
            y,
            pressure: DOWN_PRESSURE,
        }
    }

    /// Contact 0 lifted at (x, y).
    pub fn up(x: i32, y: i32) -> Self {
        Self {
            pressure: 0,
            ..Self::down(x, y)
        }
    }
}

/// Adds `authorization: Bearer <token>` to every call when a token is known.
#[derive(Clone)]
struct Bearer(Option<MetadataValue<Ascii>>);

impl Interceptor for Bearer {
    fn call(
        &mut self,
        mut request: tonic::Request<()>,
    ) -> Result<tonic::Request<()>, tonic::Status> {
        if let Some(value) = &self.0 {
            request
                .metadata_mut()
                .insert("authorization", value.clone());
        }
        Ok(request)
    }
}

type Client = EmulatorControllerClient<InterceptedService<Channel, Bearer>>;

/// Whether `status` reports a deadline: tonic's own `grpc-timeout` enforcement
/// answers `Cancelled` with "Timeout expired", the server `DeadlineExceeded`.
fn is_deadline(status: &tonic::Status) -> bool {
    status.code() == tonic::Code::DeadlineExceeded
        || (status.code() == tonic::Code::Cancelled
            && status.message() == tonic::TimeoutExpired(()).to_string())
}

/// Awaits `response` for at most `deadline`.
async fn within<T>(
    call: &'static str,
    deadline: Duration,
    response: impl Future<Output = Result<tonic::Response<T>, tonic::Status>>,
) -> Result<T, Error> {
    match tokio::time::timeout(deadline, response).await {
        Err(_) => Err(Error::Deadline {
            call,
            after: deadline,
        }),
        Ok(Err(status)) if is_deadline(&status) => Err(Error::Deadline {
            call,
            after: deadline,
        }),
        Ok(Err(status)) => Err(Error::Status { call, status }),
        Ok(Ok(response)) => Ok(response.into_inner()),
    }
}

/// A unary request that also tells the emulator its deadline (`grpc-timeout`).
fn unary<T>(message: T, deadline: Duration) -> tonic::Request<T> {
    let mut request = tonic::Request::new(message);
    request.set_timeout(deadline);
    request
}

/// An error and its sources on one line, since tonic's transport errors say
/// little on their own; a cause that repeats the one before is left out.
fn chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut last = text.clone();
    let mut source = error.source();
    while let Some(cause) = source {
        let message = cause.to_string();
        if message != last {
            text.push_str(": ");
            text.push_str(&message);
        }
        last = message;
        source = cause.source();
    }
    text
}

/// One HTTP/2 connection to `uri`, established within the connect deadline.
async fn channel(uri: &str, options: &Options) -> Result<Channel, String> {
    let endpoint = tonic::transport::Endpoint::from_shared(uri.to_owned())
        .map_err(|e| format!("bad endpoint {uri}: {}", chain(&e)))?
        .connect_timeout(options.connect_deadline)
        .tcp_nodelay(true);
    match tokio::time::timeout(options.connect_deadline, endpoint.connect()).await {
        Err(_) => Err(format!(
            "no connection to {uri} within {:?}",
            options.connect_deadline
        )),
        Ok(Err(e)) => Err(format!("connecting to {uri}: {}", chain(&e))),
        Ok(Ok(channel)) => Ok(channel),
    }
}

/// A client on `channel` that accepts messages up to [`MAX_MESSAGE`].
fn client(channel: Channel, bearer: Bearer) -> Client {
    EmulatorControllerClient::with_interceptor(channel, bearer)
        .max_decoding_message_size(MAX_MESSAGE)
        .max_encoding_message_size(MAX_MESSAGE)
}

/// A frame from an `Image` message.
fn frame_from(image: proto::Image, call: &'static str) -> Result<Frame, Error> {
    let (width, height) = image
        .format
        .as_ref()
        .map_or((0, 0), |format| (format.width, format.height));
    Frame::new(width, height, image.image)
        .map(|frame| frame.with_timing(image.seq, image.timestamp_us))
        .map_err(|error| Error::Frame { call, error })
}

/// The request for RGB888 frames of `width` by `height`, 0 by 0 for the
/// display size.
fn rgb888(width: u32, height: u32) -> proto::ImageFormat {
    proto::ImageFormat {
        format: ImgFormat::Rgb888 as i32,
        width,
        height,
        ..Default::default()
    }
}

/// A connected emulator controller.
///
/// Status, screenshots and frames use one HTTP/2 connection and touches a
/// second one, which halved drag-input p95 while native frames streamed (S7).
/// The client sends no keys: without a keyboard device the emulator accepts
/// and drops them, so [`Controller::keys_supported`] tells the driver whether
/// keys may go this way at all.
#[derive(Clone)]
pub struct Controller {
    serial: String,
    frames: Client,
    input: Client,
    hardware: Hardware,
    deadline: Duration,
}

impl fmt::Debug for Controller {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Controller")
            .field("serial", &self.serial)
            .field("hardware", &self.hardware)
            .field("deadline", &self.deadline)
            .finish()
    }
}

impl Controller {
    /// Opens both connections and reads the hardware through `getStatus`. Any
    /// failure, including a port that accepts connections and never answers,
    /// makes the endpoint [`Unavailable`].
    pub async fn connect(endpoint: &Endpoint, options: Options) -> Result<Self, Unavailable> {
        let unavailable =
            |reason: String| Unavailable::new(&endpoint.serial, endpoint.console_port, reason);
        let bearer = match &endpoint.token {
            None => Bearer(None),
            Some(token) => Bearer(Some(
                format!("Bearer {token}")
                    .parse::<MetadataValue<Ascii>>()
                    .map_err(|_| unavailable("the token is not a valid header value".into()))?,
            )),
        };
        let frames = client(
            channel(&endpoint.uri, &options)
                .await
                .map_err(unavailable)?,
            bearer.clone(),
        );
        let input = client(
            channel(&endpoint.uri, &options)
                .await
                .map_err(unavailable)?,
            bearer,
        );
        let status = within(
            "getStatus",
            options.deadline,
            frames.clone().get_status(unary((), options.deadline)),
        )
        .await
        .map_err(|e| unavailable(e.to_string()))?;
        let hardware = Hardware::from_status(&status).map_err(unavailable)?;
        Ok(Self {
            serial: endpoint.serial.clone(),
            frames,
            input,
            hardware,
            deadline: options.deadline,
        })
    }

    pub fn serial(&self) -> &str {
        &self.serial
    }

    /// The display size and keyboard flag read at connection time.
    pub fn hardware(&self) -> Hardware {
        self.hardware
    }

    /// Whether keys and text may be sent through the controller: only when the
    /// AVD declares a keyboard (`hw.keyboard=yes`).
    pub fn keys_supported(&self) -> bool {
        self.hardware.keyboard
    }

    /// Sends one touch event holding `points` on the main display and returns
    /// the round trip. The emulator acknowledges once it has scheduled the
    /// event, not when the app received it.
    pub async fn touch(&self, points: &[TouchPoint]) -> Result<Duration, Error> {
        let event = proto::TouchEvent {
            touches: points
                .iter()
                .map(|p| proto::Touch {
                    x: p.x,
                    y: p.y,
                    identifier: p.id,
                    pressure: p.pressure,
                    ..Default::default()
                })
                .collect(),
            display: 0,
        };
        let started = Instant::now();
        within(
            "sendTouch",
            self.deadline,
            self.input.clone().send_touch(unary(event, self.deadline)),
        )
        .await?;
        Ok(started.elapsed())
    }

    /// Presses and lifts contact 0 at (x, y), in display pixels.
    pub async fn tap(&self, x: i32, y: i32) -> Result<(), Error> {
        self.touch(&[TouchPoint::down(x, y)]).await?;
        self.touch(&[TouchPoint::up(x, y)]).await?;
        Ok(())
    }

    /// A raw RGB888 capture at the display's full resolution.
    pub async fn screenshot(&self) -> Result<Frame, Error> {
        let call = "getScreenshot";
        let image = within(
            call,
            self.deadline,
            self.frames
                .clone()
                .get_screenshot(unary(rgb888(0, 0), self.deadline)),
        )
        .await?;
        frame_from(image, call)
    }

    /// Subscribes to RGB888 frames scaled by the emulator to `working_width`
    /// pixels (0 for the display size). The emulator sends a frame whenever the
    /// guest posts one, changed or not, and nothing while the screen is still.
    pub async fn frames(&self, working_width: u32) -> Result<FrameStream, Error> {
        let size = scaled_size(self.hardware.display, working_width);
        // No grpc-timeout here: it would end the stream.
        let request = tonic::Request::new(rgb888(size.0, size.1));
        let inner = within(
            "streamScreenshot",
            self.deadline,
            self.frames.clone().stream_screenshot(request),
        )
        .await?;
        Ok(FrameStream { inner, size })
    }
}

/// A subscription to the emulator's frames.
pub struct FrameStream {
    inner: tonic::Streaming<proto::Image>,
    size: (u32, u32),
}

impl FrameStream {
    /// The size requested from the emulator, `(0, 0)` for the display size.
    pub fn requested_size(&self) -> (u32, u32) {
        self.size
    }

    /// The next frame, or `None` if none arrives within `wait`, which happens
    /// whenever the screen is still.
    pub async fn next(&mut self, wait: Duration) -> Result<Option<Frame>, Error> {
        let call = "streamScreenshot";
        match tokio::time::timeout(wait, self.inner.message()).await {
            Err(_) => Ok(None),
            Ok(Err(status)) => Err(Error::Status { call, status }),
            Ok(Ok(None)) => Err(Error::StreamEnded),
            Ok(Ok(Some(image))) => frame_from(image, call).map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;
    use tokio::net::TcpListener;

    fn endpoint(port: u16, token: Option<&str>) -> Endpoint {
        Endpoint {
            serial: "emulator-5554".into(),
            uri: format!("http://127.0.0.1:{port}"),
            token: token.map(str::to_owned),
            console_port: Some(5554),
            discovery_file: None,
        }
    }

    fn quick() -> Options {
        Options {
            deadline: Duration::from_millis(300),
            connect_deadline: Duration::from_millis(300),
        }
    }

    /// A port that accepts connections and never answers, like the emulator
    /// ports S7 found; returns it with the count of accepted connections.
    async fn silent_port() -> (u16, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicUsize::new(0));
        let count = accepted.clone();
        tokio::spawn(async move {
            let mut held = Vec::new();
            while let Ok((socket, _)) = listener.accept().await {
                count.fetch_add(1, Ordering::SeqCst);
                held.push(socket);
            }
        });
        (port, accepted)
    }

    #[tokio::test]
    async fn a_silent_endpoint_fails_at_the_deadline() {
        let (port, _) = silent_port().await;
        let started = Instant::now();
        let err = Controller::connect(&endpoint(port, None), quick())
            .await
            .unwrap_err();
        let elapsed = started.elapsed();
        assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
        assert!(err.reason.contains("getStatus"), "{}", err.reason);
        assert!(err.reason.contains("no answer within"), "{}", err.reason);
        assert!(err.warning().contains("-grpc 8554 -grpc-use-token"));
    }

    #[tokio::test]
    async fn input_travels_on_a_second_connection() {
        let (port, accepted) = silent_port().await;
        let _ = Controller::connect(&endpoint(port, None), quick()).await;
        assert_eq!(accepted.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_closed_port_is_unavailable_with_the_fallback_warning() {
        let port = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap().port()
        };
        let err = Controller::connect(&endpoint(port, Some("token")), quick())
            .await
            .unwrap_err();
        assert!(
            err.reason.contains(&format!("127.0.0.1:{port}")),
            "{}",
            err.reason
        );
        let warning = err.warning();
        assert!(warning.contains("-grpc 8554 -grpc-use-token"), "{warning}");
        assert!(warning.contains("`grpc 8554`"), "{warning}");
    }

    #[tokio::test]
    async fn a_token_that_is_no_header_value_is_refused_before_connecting() {
        let (port, accepted) = silent_port().await;
        let err = Controller::connect(&endpoint(port, Some("bad\ntoken")), quick())
            .await
            .unwrap_err();
        assert!(err.reason.contains("token"), "{}", err.reason);
        assert_eq!(accepted.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn touch_points_press_and_lift_contact_zero() {
        assert_eq!(
            TouchPoint::down(300, 60),
            TouchPoint {
                id: 0,
                x: 300,
                y: 60,
                pressure: DOWN_PRESSURE
            }
        );
        assert_eq!(TouchPoint::up(300, 60).pressure, 0);
    }
}
