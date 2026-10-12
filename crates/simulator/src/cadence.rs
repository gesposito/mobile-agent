//! The pace of typed keys (simulator-driver spec, "Text entry"): the pause
//! between one key press and the next, slower for the fields of a React
//! Native screen than for native ones (design D42 of `rn-app-fixes`).

use std::time::Duration;

use protocol::snapshot::Snapshot;

/// The prefix of React Native's own classes on iOS: its root and surface
/// views (`RCTSurfaceHostingProxyRootView`, `RCTRootComponentView`), its
/// component views (`RCTViewComponentView`, `RCTTextInputComponentView`)
/// and the views of its older architecture (`RCTRootView`, `RCTView`).
const REACT_NATIVE_PREFIX: &str = "RCT";

/// The pause between two keys `type_text_at` types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cadence {
    /// From one key's release to the next key's press.
    pub gap: Duration,
}

impl Cadence {
    /// Native fields, UIKit's and SwiftUI's. Spike S3 typed exactly with
    /// 60 ms; this shorter gap kept every character and its order in the
    /// fixtures' fields.
    pub const NATIVE: Cadence = Cadence {
        gap: Duration::from_millis(15),
    };

    /// React Native fields, which take keys through the app's JavaScript
    /// thread: a field whose value the app owns, on a screen whose render
    /// takes longer than the gap, loses keys. Task 3.5 of `rn-app-fixes`
    /// chose it from 15, 30 and 60 ms as the slowest that lost none.
    pub const REACT_NATIVE: Cadence = Cadence {
        gap: Duration::from_millis(60),
    };

    /// The cadence for the fields of `snapshot`, a read of this driver's:
    /// React Native's when one of its classes is React Native's, the native
    /// one otherwise. The reader names an element the runtime leaves untyped
    /// by its platform class, and React Native draws its screens under its
    /// own views, so a read of a React Native screen names some. The string
    /// table holds each class once, beside the bundle identifier, so no node
    /// is visited. A native field on a React Native screen, such as an
    /// alert's, is typed at React Native's cadence too, which costs time and
    /// loses nothing; the translator names no platform class, so its reads
    /// keep the native cadence.
    pub fn for_read(snapshot: &Snapshot) -> Cadence {
        let react_native = snapshot
            .strings
            .iter()
            .any(|name| name.starts_with(REACT_NATIVE_PREFIX));
        if react_native {
            Cadence::REACT_NATIVE
        } else {
            Cadence::NATIVE
        }
    }
}

impl Default for Cadence {
    fn default() -> Cadence {
        Cadence::NATIVE
    }
}

#[cfg(test)]
mod tests {
    use simulator_reader::elements::{self, Element, Limits, Owner, Read};

    use super::*;

    /// An element the runtime typed `automation_type` (0 for none, 2 for a
    /// window, 49 for a text field), of the platform class `class`.
    fn element(automation_type: u64, class: &str, children: Vec<Element>) -> Element {
        Element {
            automation_type: Some(automation_type),
            class: Some(class.into()),
            frame: Some([0.0, 0.0, 402.0, 874.0]),
            children,
            ..Element::default()
        }
    }

    /// Each element of `chain` holding the next, the first outermost.
    fn nested(chain: Vec<Element>) -> Element {
        chain
            .into_iter()
            .rev()
            .reduce(|inner, mut outer| {
                outer.children.push(inner);
                outer
            })
            .expect("a chain of at least one element")
    }

    /// A read of a login screen's username field below `views`, the views
    /// from the window's root view down, as the reader returned it on iOS
    /// 26.5, every branch but the field's trimmed.
    fn login(bundle: &str, views: &[&str]) -> Snapshot {
        let field = Element {
            identifier: Some("login-username".into()),
            label: Some("Username".into()),
            frame: Some([24.0, 86.0, 354.0, 44.0]),
            ..element(49, "UITextField", Vec::new())
        };
        let mut chain = vec![
            element(1, "UIApplication", Vec::new()),
            element(2, "UIWindow", Vec::new()),
            element(0, "UITransitionView", Vec::new()),
            element(0, "UIDropShadowView", Vec::new()),
        ];
        chain.extend(views.iter().map(|class| element(0, class, Vec::new())));
        chain.push(field);
        elements::snapshot(&Read {
            owner: Owner {
                pid: 4242,
                bundle_id: Some(bundle.into()),
                name: "Fixture".into(),
                system: false,
            },
            root: nested(chain),
            scale: 3.0,
            limits: Limits::default(),
            depth_cut: false,
        })
    }

    /// Task 3.5 of `rn-app-fixes`: the Expo fixture's username field lies
    /// under React Native's root and surface views and its own text input
    /// view, which the reader names by their classes, so its keys go at
    /// React Native's cadence, slower than the native one.
    #[test]
    fn a_read_of_react_natives_views_types_at_the_react_native_cadence() {
        let read = login(
            "dev.mobileagent.fixture.expo",
            &[
                "RCTSurfaceHostingProxyRootView",
                "RCTSurfaceView",
                "RCTRootComponentView",
                "RNCSafeAreaProviderComponentView",
                "RNCSafeAreaViewComponentView",
                "RCTTextInputComponentView",
            ],
        );
        assert_eq!(Cadence::for_read(&read), Cadence::REACT_NATIVE);
        assert!(
            Cadence::REACT_NATIVE.gap > Cadence::NATIVE.gap,
            "React Native's cadence is the slower: {:?} against {:?}",
            Cadence::REACT_NATIVE,
            Cadence::NATIVE
        );
    }

    /// The SwiftUI fixture's username field keeps the native cadence.
    #[test]
    fn a_native_read_types_at_the_native_cadence() {
        let read = login(
            "dev.mobileagent.fixture.swiftui",
            &[
                "_TtGC7SwiftUI14_UIHostingViewGVS_15ModifiedContentVS_7AnyViewVS_12RootModifier__",
                "SwiftUI.AccessibilityNode",
            ],
        );
        assert_eq!(Cadence::for_read(&read), Cadence::NATIVE);
        assert_eq!(Cadence::NATIVE.gap, Duration::from_millis(15));
    }
}
