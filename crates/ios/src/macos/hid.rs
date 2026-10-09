//! Input from the host through the developer disk image's `dtuhidd` over
//! RSD (design D22), as spike S11 sent it with `idevice`'s HID clients:
//! touches as 58-byte `mainTouchscreen` reports to
//! `com.apple.coredevice.hid.universalhidservice`, hardware buttons and keys
//! as Indigo events to `com.apple.coredevice.hid.indigo`, and a virtual
//! keyboard the host registers on the universal service for typing, removed
//! afterwards (Xcode's notes say a device may otherwise stay in hardware
//! keyboard mode for two minutes). Each input is acknowledged once its last
//! report is written to the tunnel; on iOS 26.6.2 they land with no display
//! stream running (S11). Both connections are kept for the target's
//! lifetime.

use std::time::{Duration, Instant};

use idevice::ReadWrite;
use idevice::core_device::{
    ButtonState, DIGITIZER_SURFACE_MAIN_TOUCHSCREEN, IndigoHidClient, KeyboardUsage,
    TOUCHSCREEN_STATE_CONTACT, TOUCHSCREEN_STATE_RELEASE, UniversalHidServiceClient,
    build_touchscreen_report,
};

use super::driver::Target;
use super::host::{Host, describe};
use crate::error::{Error, Result};
use crate::model::{Button, Dispatch, InputPath, Key};
use crate::touch::{Contact, TouchReport};

/// How long a hardware button is held: `idevice`'s tool and S11 held home
/// 80 ms.
pub(crate) const BUTTON_HOLD: Duration = Duration::from_millis(80);
/// How long a key is held, and the pause after it: S11 typed this way.
pub(crate) const KEY_HOLD: Duration = Duration::from_millis(30);
/// The HID Consumer page, where the hardware buttons' usages are.
const CONSUMER_PAGE: u64 = 0x0C;

type Universal = UniversalHidServiceClient<Box<dyn ReadWrite>>;
type Indigo = IndigoHidClient<Box<dyn ReadWrite>>;

/// The two connections to `dtuhidd` a target keeps, each opened on first
/// use.
#[derive(Default)]
pub(crate) struct HidClients {
    universal: Option<Universal>,
    indigo: Option<Indigo>,
}

impl HidClients {
    /// The services open, by RSD name.
    pub(crate) fn open(&self) -> Vec<&'static str> {
        let mut open = Vec::new();
        if self.indigo.is_some() {
            open.push(crate::services::HID_INDIGO);
        }
        if self.universal.is_some() {
            open.push(crate::services::HID_UNIVERSAL);
        }
        open
    }

    pub(crate) fn close(&mut self) {
        self.universal = None;
        self.indigo = None;
    }
}

/// One input, as `dtuhidd` takes it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum HidInput {
    Touches(Vec<TouchReport>),
    Button(Button),
    Key(Key),
    Keys(Vec<Key>),
}

/// A write that failed after `written` reports reached the device.
struct Failed {
    written: usize,
    cause: String,
}

/// When the reports went out, from the request.
struct Sent {
    reports: usize,
    first: Duration,
    last: Duration,
}

impl<H: Host> Target<H> {
    /// Sends `input` through `dtuhidd`, timed from `requested`. A write that
    /// fails before any report reached the device goes again on new
    /// connections, through a tunnel opened again if it dropped; one that
    /// fails after does not, since the device saw part of it.
    pub(crate) async fn send_hid(
        &mut self,
        input: &HidInput,
        requested: Instant,
    ) -> Result<Dispatch> {
        let mut retried = false;
        loop {
            let sent = match input {
                HidInput::Touches(reports) => {
                    let client = self.universal().await?;
                    touches(client, reports, requested).await
                }
                HidInput::Button(button) => {
                    let client = self.indigo().await?;
                    press_button(client, *button, requested).await
                }
                HidInput::Key(key) => {
                    let client = self.indigo().await?;
                    press_key(client, *key, requested).await
                }
                HidInput::Keys(keys) => {
                    let client = self.universal().await?;
                    type_keys(client, keys, requested).await
                }
            };
            match sent {
                Ok(sent) => {
                    return Ok(Dispatch {
                        path: InputPath::Hid,
                        reports: sent.reports,
                        first: sent.first,
                        acknowledged: sent.last,
                    });
                }
                Err(failed) => {
                    self.hid.close();
                    if failed.written == 0 && !retried {
                        retried = true;
                        continue;
                    }
                    return Err(Error::Disconnected {
                        target: self.target.clone(),
                        cause: format!(
                            "dtuhidd's connection failed after {} reports: {}",
                            failed.written, failed.cause
                        ),
                    });
                }
            }
        }
    }

    /// The universal HID service's connection, opened unless it is open.
    async fn universal(&mut self) -> Result<&mut Universal> {
        if self.hid.universal.is_none() {
            self.hid.universal = Some(self.tunnel.connect::<Universal>().await?);
        }
        Ok(self.hid.universal.as_mut().expect("just opened"))
    }

    /// The Indigo service's connection, opened unless it is open.
    async fn indigo(&mut self) -> Result<&mut Indigo> {
        if self.hid.indigo.is_none() {
            self.hid.indigo = Some(self.tunnel.connect::<Indigo>().await?);
        }
        Ok(self.hid.indigo.as_mut().expect("just opened"))
    }
}

/// Writes `reports` to the main touchscreen, each at its time.
async fn touches(
    client: &mut Universal,
    reports: &[TouchReport],
    requested: Instant,
) -> std::result::Result<Sent, Failed> {
    let origin = tokio::time::Instant::now();
    let mut first = None;
    for (written, report) in reports.iter().enumerate() {
        tokio::time::sleep_until(origin + report.at).await;
        let state = match report.contact {
            Contact::Touching => TOUCHSCREEN_STATE_CONTACT,
            Contact::Lifted => TOUCHSCREEN_STATE_RELEASE,
        };
        client
            .send_report(
                DIGITIZER_SURFACE_MAIN_TOUCHSCREEN,
                build_touchscreen_report(state, report.x, report.y, None),
            )
            .await
            .map_err(|error| Failed {
                written,
                cause: describe(&error),
            })?;
        first.get_or_insert_with(|| requested.elapsed());
    }
    Ok(Sent {
        reports: reports.len(),
        first: first.unwrap_or_default(),
        last: requested.elapsed(),
    })
}

/// A hardware button down, held [`BUTTON_HOLD`], and up.
async fn press_button(
    client: &mut Indigo,
    button: Button,
    requested: Instant,
) -> std::result::Result<Sent, Failed> {
    let usage = button.usage();
    let failed = |written| {
        move |error: idevice::IdeviceError| Failed {
            written,
            cause: describe(&error),
        }
    };
    client
        .send_button(CONSUMER_PAGE, usage, ButtonState::Down)
        .await
        .map_err(failed(0))?;
    let first = requested.elapsed();
    tokio::time::sleep(BUTTON_HOLD).await;
    client
        .send_button(CONSUMER_PAGE, usage, ButtonState::Up)
        .await
        .map_err(failed(1))?;
    Ok(Sent {
        reports: 2,
        first,
        last: requested.elapsed(),
    })
}

/// A key with its modifiers held around it, as Indigo keyboard events.
async fn press_key(
    client: &mut Indigo,
    key: Key,
    requested: Instant,
) -> std::result::Result<Sent, Failed> {
    let modifiers = key.modifiers.usages();
    let mut events: Vec<(u16, ButtonState)> = modifiers
        .iter()
        .map(|usage| (*usage, ButtonState::Down))
        .collect();
    events.push((key.usage, ButtonState::Down));
    events.push((key.usage, ButtonState::Up));
    events.extend(
        modifiers
            .iter()
            .rev()
            .map(|usage| (*usage, ButtonState::Up)),
    );
    let mut first = None;
    for (written, (usage, state)) in events.iter().enumerate() {
        if *usage == key.usage && *state == ButtonState::Up {
            tokio::time::sleep(KEY_HOLD).await;
        }
        client
            .send_keyboard(u64::from(*usage), *state)
            .await
            .map_err(|error| Failed {
                written,
                cause: describe(&error),
            })?;
        first.get_or_insert_with(|| requested.elapsed());
    }
    Ok(Sent {
        reports: events.len(),
        first: first.unwrap_or_default(),
        last: requested.elapsed(),
    })
}

/// `keys` typed on a virtual keyboard registered for them: each key with
/// its modifiers in one report, held [`KEY_HOLD`], then none pressed, with
/// a pause; the keyboard is removed afterwards, whatever happened.
async fn type_keys(
    client: &mut Universal,
    keys: &[Key],
    requested: Instant,
) -> std::result::Result<Sent, Failed> {
    let mut keyboard = client
        .create_main_keyboard()
        .await
        .map_err(|error| Failed {
            written: 0,
            cause: format!("registering the virtual keyboard: {error}"),
        })?;
    let mut first = None;
    let mut written = 0;
    let mut failure = None;
    for key in keys {
        let usages: std::result::Result<Vec<KeyboardUsage>, _> = key
            .modifiers
            .usages()
            .into_iter()
            .chain(std::iter::once(key.usage))
            .map(KeyboardUsage::new)
            .collect();
        let usages = match usages {
            Ok(usages) => usages,
            Err(error) => {
                failure = Some(format!("usage {:#x}: {error}", key.usage));
                break;
            }
        };
        if let Err(error) = client.set_main_keyboard_usages(&mut keyboard, usages).await {
            failure = Some(format!("a key's report: {error}"));
            break;
        }
        first.get_or_insert_with(|| requested.elapsed());
        written += 1;
        tokio::time::sleep(KEY_HOLD).await;
        if let Err(error) = client.set_main_keyboard_usages(&mut keyboard, []).await {
            failure = Some(format!("a key's release: {error}"));
            break;
        }
        written += 1;
        tokio::time::sleep(KEY_HOLD).await;
    }
    let last = requested.elapsed();
    let removed = client.remove_main_keyboard(&mut keyboard).await;
    if let Some(cause) = failure {
        return Err(Failed { written, cause });
    }
    removed.map_err(|error| Failed {
        written,
        cause: format!("removing the virtual keyboard: {error}"),
    })?;
    Ok(Sent {
        reports: written,
        first: first.unwrap_or_default(),
        last,
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use plist::Value;

    use super::*;
    use crate::discovery::tests::recorded_images;
    use crate::macos::driver::Gesture;
    use crate::macos::driver::tests::{FakeHost, PHONE, core};
    use crate::macos::fake_xpc::{HidEvent, TimedHid};
    use crate::model::{
        AttachOptions, Modifiers, Point, RunnerSpec, Screen, Swipe, TargetStatus, Trust,
    };
    use crate::services;
    use crate::touch::{self, REST, TAP_HOLD};

    const SE: Screen = Screen {
        width: 750,
        height: 1334,
        scale: 2.0,
    };
    /// What a report may take to cross the fake tunnel and be read.
    const SLACK: Duration = Duration::from_millis(40);
    const TOUCHING: u8 = 0xC2;
    const LIFTED: u8 = 0x02;

    /// The events `dtuhidd` took, once `count` of them arrived or a second
    /// passed.
    fn events(host: &FakeHost, count: usize) -> Vec<TimedHid> {
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let events = host.device.hid_events();
            if events.len() >= count || Instant::now() >= deadline {
                return events;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    fn kinds(events: &[TimedHid]) -> Vec<HidEvent> {
        events.iter().map(|timed| timed.event.clone()).collect()
    }

    fn at(point: Point) -> (u16, u16) {
        touch::normalize(&SE, point).expect("on the screen")
    }

    #[test]
    fn a_tap_is_a_main_touchscreen_contact_held_then_released_and_acknowledged_at_the_release() {
        let core = core(FakeHost::ready());
        let point = Point::new(187.0, 550.0);
        let dispatch = core
            .input(PHONE, Gesture::Press(point, TAP_HOLD))
            .expect("tapped");
        assert_eq!(dispatch.path, InputPath::Hid);
        assert_eq!(dispatch.reports, 2);
        assert!(dispatch.acknowledged >= TAP_HOLD, "{dispatch:?}");
        assert!(
            dispatch.acknowledged - dispatch.first < TAP_HOLD + SLACK,
            "acknowledged as the release is written, with no wait after it: {dispatch:?}"
        );
        let events = events(&core.host, 2);
        let (x, y) = at(point);
        assert_eq!(
            kinds(&events),
            vec![
                HidEvent::Touch {
                    state: TOUCHING,
                    x,
                    y
                },
                HidEvent::Touch {
                    state: LIFTED,
                    x,
                    y
                },
            ]
        );
        let held = events[1].at - events[0].at;
        assert!(held + Duration::from_millis(5) >= TAP_HOLD, "held {held:?}");
    }

    #[test]
    fn a_long_press_holds_its_contact_for_its_hold() {
        let core = core(FakeHost::ready());
        let hold = Duration::from_millis(300);
        let dispatch = core
            .input(PHONE, Gesture::Press(Point::new(100.0, 100.0), hold))
            .expect("pressed");
        assert!(dispatch.acknowledged >= hold, "{dispatch:?}");
        let events = events(&core.host, 2);
        assert!(events[1].at - events[0].at + Duration::from_millis(5) >= hold);
    }

    #[test]
    fn a_resting_swipe_rests_before_it_moves_one_pixel_on_and_lifts() {
        let core = core(FakeHost::ready());
        let stroke = Swipe::new(
            Point::new(187.0, 500.0),
            Point::new(187.0, 260.0),
            Duration::from_millis(160),
        )
        .resting();
        let dispatch = core.input(PHONE, Gesture::Swipe(stroke)).expect("swiped");
        assert!(
            dispatch.acknowledged >= stroke.duration + REST,
            "{dispatch:?}"
        );
        let expected = touch::swipe(&SE, &stroke).expect("the plan");
        let events = events(&core.host, expected.len());
        assert_eq!(events.len(), expected.len());
        let end = at(stroke.to);
        let arrived = events
            .iter()
            .position(|timed| matches!(timed.event, HidEvent::Touch { x, y, .. } if (x, y) == end))
            .expect("the finger reaches the end");
        let (lift, crept) = (&events[events.len() - 1], &events[events.len() - 2]);
        assert!(
            crept.at - events[arrived].at + Duration::from_millis(5) >= REST,
            "it rests {REST:?} at the end before moving on"
        );
        match (&crept.event, &lift.event) {
            (
                HidEvent::Touch {
                    state: TOUCHING,
                    x: crept_x,
                    y: crept_y,
                },
                HidEvent::Touch {
                    state: LIFTED,
                    x,
                    y,
                },
            ) => {
                assert_eq!((crept_x, crept_y), (x, y), "it lifts where it crept to");
                assert!(*crept_y < end.1, "one pixel on, up the screen");
            }
            other => panic!("expected a move then the lift: {other:?}"),
        }
    }

    #[test]
    fn a_moving_swipe_lifts_at_the_end_of_its_path() {
        let core = core(FakeHost::ready());
        let stroke = Swipe::new(
            Point::new(187.0, 500.0),
            Point::new(187.0, 260.0),
            Duration::from_millis(100),
        );
        core.input(PHONE, Gesture::Swipe(stroke)).expect("swiped");
        let expected = touch::swipe(&SE, &stroke).expect("the plan");
        let events = events(&core.host, expected.len());
        let (x, y) = at(stroke.to);
        assert_eq!(
            events.last().map(|timed| timed.event.clone()),
            Some(HidEvent::Touch {
                state: LIFTED,
                x,
                y
            })
        );
    }

    #[test]
    fn the_home_button_is_an_indigo_button_event_down_held_then_up() {
        let core = core(FakeHost::ready());
        let dispatch = core
            .input(PHONE, Gesture::Button(Button::Home))
            .expect("pressed");
        assert!(dispatch.acknowledged >= BUTTON_HOLD, "{dispatch:?}");
        let events = events(&core.host, 2);
        assert_eq!(
            kinds(&events),
            vec![
                HidEvent::Button {
                    page: 0x0C,
                    usage: 0x40,
                    down: true
                },
                HidEvent::Button {
                    page: 0x0C,
                    usage: 0x40,
                    down: false
                },
            ]
        );
        assert!(events[1].at - events[0].at + Duration::from_millis(5) >= BUTTON_HOLD);
    }

    #[test]
    fn lock_and_volume_take_their_consumer_usages() {
        let core = core(FakeHost::ready());
        for (button, usage) in [
            (Button::Lock, 0x30),
            (Button::VolumeUp, 0xE9),
            (Button::VolumeDown, 0xEA),
        ] {
            core.input(PHONE, Gesture::Button(button)).expect("pressed");
            let events = events(&core.host, 2);
            assert!(
                events.iter().any(|timed| timed.event
                    == HidEvent::Button {
                        page: 0x0C,
                        usage,
                        down: true
                    }),
                "{button:?}: {:?}",
                kinds(&events)
            );
        }
    }

    #[test]
    fn a_key_is_an_indigo_keyboard_event_with_its_modifiers_held_around_it() {
        let core = core(FakeHost::ready());
        let key = Key {
            usage: 0x04,
            modifiers: Modifiers {
                shift: true,
                ..Modifiers::default()
            },
        };
        let dispatch = core.input(PHONE, Gesture::Key(key)).expect("pressed");
        assert_eq!(dispatch.reports, 4);
        let key_event = |usage, down| HidEvent::IndigoKey { usage, down };
        assert_eq!(
            kinds(&events(&core.host, 4)),
            vec![
                key_event(0xE1, true),
                key_event(0x04, true),
                key_event(0x04, false),
                key_event(0xE1, false),
            ]
        );
    }

    #[test]
    fn keys_are_typed_on_a_virtual_keyboard_registered_then_removed() {
        let core = core(FakeHost::ready());
        let keys = [Key::usage(0x04), Key::usage(0x05)];
        let dispatch = core
            .input(PHONE, Gesture::Keys(keys.to_vec()))
            .expect("typed");
        assert_eq!(dispatch.path, InputPath::Hid);
        assert_eq!(dispatch.reports, 4, "each key pressed, then released");
        let events = kinds(&events(&core.host, 6));
        let HidEvent::KeyboardCreated(id) = events[0] else {
            panic!("the keyboard is registered first: {events:?}");
        };
        let pressed = |usages: &[u16]| HidEvent::KeyboardReport {
            pressed: usages.to_vec(),
        };
        assert_eq!(
            events,
            vec![
                HidEvent::KeyboardCreated(id),
                pressed(&[0x04]),
                pressed(&[]),
                pressed(&[0x05]),
                pressed(&[]),
                HidEvent::KeyboardRemoved(id),
            ]
        );
        assert_eq!(
            core.host.device.keyboard(id),
            None,
            "nothing left registered"
        );
    }

    #[test]
    fn a_point_off_the_screen_is_refused_before_any_report() {
        let core = core(FakeHost::ready());
        let error = core
            .input(PHONE, Gesture::Press(Point::new(400.0, 10.0), TAP_HOLD))
            .expect_err("off the screen");
        assert_eq!(error.code(), "invalid_argument", "{error}");
        std::thread::sleep(Duration::from_millis(50));
        assert!(core.host.device.hid_events().is_empty());
    }

    #[test]
    fn the_connections_to_dtuhidd_are_kept_and_opened_again_after_the_tunnel_dropped() {
        let core = core(FakeHost::ready());
        for _ in 0..3 {
            core.input(PHONE, Gesture::Press(Point::new(50.0, 50.0), TAP_HOLD))
                .expect("tapped");
        }
        let universal = core
            .host
            .device
            .port(services::HID_UNIVERSAL)
            .expect("listed");
        let opened = |core: &crate::macos::driver::Core<FakeHost>| {
            core.host
                .device
                .connections()
                .iter()
                .filter(|port| **port == universal)
                .count()
        };
        assert_eq!(opened(&core), 1, "one connection for every tap");
        let status: TargetStatus = core.status(PHONE).expect("status");
        assert!(
            status.services_open.contains(&services::HID_UNIVERSAL),
            "{status:?}"
        );
        core.host.device.drop_tunnel();
        core.input(PHONE, Gesture::Press(Point::new(50.0, 50.0), TAP_HOLD))
            .expect("tapped through a new tunnel");
        assert_eq!(opened(&core), 2);
        assert_eq!(core.host.device.opens(), 2);
    }

    #[test]
    fn an_ios_whose_dtuhidd_takes_no_report_goes_through_the_runner_or_is_refused() {
        let host = FakeHost::with(
            &[("ProductVersion", Value::String("17.6".into()))],
            Trust::Trusted,
            Some(Ok(recorded_images())),
        );
        host.device.install_runner("0.1.0");
        let core = core(host);
        let error = core
            .input(PHONE, Gesture::Press(Point::new(50.0, 50.0), TAP_HOLD))
            .expect_err("no runner attached");
        assert_eq!(error.code(), "input_unavailable", "{error}");
        assert!(error.to_string().contains("17.6"), "{error}");
        core.attach(PHONE, &AttachOptions::new(RunnerSpec::new("0.1.0")))
            .expect("attaches");
        let dispatch = core
            .input(PHONE, Gesture::Press(Point::new(50.0, 50.0), TAP_HOLD))
            .expect("synthesized by the runner");
        assert_eq!(dispatch.path, InputPath::Runner);
        assert_eq!(core.host.device.runner_served(), vec!["tap"]);
        assert!(
            core.host.device.hid_events().is_empty(),
            "dtuhidd was not used"
        );
    }
}
