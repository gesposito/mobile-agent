//! The Simulator runtime's private accessibility client, resolved by name
//! inside the guest: agent-device's `SnapshotBridgeRuntime.m` in Rust, as
//! spike S4 ported it.
//!
//! - XCTAutomationSupport: `XCTAccessibilityFramework -initForRemoteAccess`
//!   and `-userTestingSnapshotForElement:options:error:`, for the tree and
//!   for each scroll view's scroll area,
//!   `XCAccessibilityElement +elementWithProcessIdentifier:` and
//!   `-AXUIElement`, and the C functions `XCTDefaultSnapshotParameters` and
//!   `XCAXAccessibilityAttributesForStringAttributes`.
//! - AXRuntime: `+[AXElement primaryApp]` for foreground ownership, and
//!   `AXValueGetTypeID` and `AXValueGetValue` to unpack frames.
//! - libAccessibility: `_AXSAutomationEnabled`, and AccessibilityUtilities'
//!   `-[AXSettings setAutomationEnabled:]` to assert automation mode before
//!   every read.

use std::ffi::{CStr, c_void};

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{msg_send, sel};
use objc2_foundation::{NSArray, NSError, NSNumber, NSString};

use crate::codes;
use crate::elements::{Limits, Owner, Read};
use crate::guest::extract;
use crate::guest::objc::{self, AXUIElementRef, Failure, guarded};
use crate::guest::process;

pub const AX_RUNTIME: &str = "/System/Library/PrivateFrameworks/AXRuntime.framework/AXRuntime";
pub const XCT_AUTOMATION_SUPPORT: &str =
    "/Developer/Library/PrivateFrameworks/XCTAutomationSupport.framework/XCTAutomationSupport";
pub const ACCESSIBILITY_UTILITIES: &str =
    "/System/Library/PrivateFrameworks/AccessibilityUtilities.framework/AccessibilityUtilities";

/// The attributes requested per node, agent-device's list. The placeholder
/// is optional: a runtime without it is read without placeholders.
pub const ATTRIBUTE_NAMES: [&str; 10] = [
    "XC_kAXXCAttributeElementType",
    "XC_kAXXCAttributeElementBaseType",
    "XC_kAXXCAttributeLabel",
    "XC_kAXXCAttributeValue",
    "XC_kAXXCAttributePlaceholderValue",
    "XC_kAXXCAttributeIdentifier",
    "XC_kAXXCAttributeFrame",
    "XC_kAXXCAttributeAutomationType",
    "XC_kAXXCAttributeTraits",
    "XC_kAXXCAttributeChildren",
];

/// The attributes asked of each scroll view on screen, in a snapshot of
/// its own element: where its frame's edges fall in its content along each
/// axis, and the part of its frame that shows content. Asked of every node,
/// they cost the SwiftUI Big screen's 546 nodes 40 ms a read; asked of the
/// SwiftUI list's one scroll view, 3.6 ms. A runtime that does not number
/// them is read without scroll semantics.
pub const SCROLL_ATTRIBUTE_NAMES: [&str; 3] = [
    "XC_kAXXCAttributeVisibleVerticalScrollArea",
    "XC_kAXXCAttributeVisibleHorizontalScrollArea",
    "XC_kAXXCAttributeVisibleFrame",
];

/// The attribute asked of each dimming view, in a snapshot of its own
/// element: whether it takes touches, which tells a sheet that blocks the
/// screen from one at an undimmed detent. agent-device's bridge asks it of
/// dimming views alone for the same reason as the scroll attributes: asked
/// of every node, it costs a read far more.
pub const INTERACTION_ATTRIBUTE_NAME: &str = "XC_kAXXCAttributeIsUserInteractionEnabled";

/// `kAXErrorIllegalArgument` from the AX server, which rejects a snapshot
/// deeper than it will serialize; XCTAutomationSupport reports the same
/// rejection as its error 5.
const AX_ERROR_REJECTED: i64 = -25201;
/// `kAXErrorCannotComplete`: the application did not answer.
const AX_ERROR_CANNOT_COMPLETE: i64 = -25204;
const XCTAS_ERROR_DOMAIN: &str = "com.apple.dt.xctest.automation-support.error";

type DefaultParametersFn = unsafe extern "C" fn() -> *mut AnyObject;
type AttributesForNamesFn = unsafe extern "C" fn(*mut AnyObject) -> *mut AnyObject;
pub(crate) type ValueGetValueFn = unsafe extern "C" fn(*const c_void, u32, *mut c_void) -> u8;
type TypeIdFn = unsafe extern "C" fn() -> usize;
type BoolFn = unsafe extern "C" fn() -> u8;
type SetBoolFn = unsafe extern "C" fn(u8);

/// A read that failed, as the protocol reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadError {
    pub code: &'static str,
    pub message: String,
    pub retryable: bool,
}

impl ReadError {
    pub fn new(code: &'static str, message: impl Into<String>, retryable: bool) -> ReadError {
        ReadError {
            code,
            message: message.into(),
            retryable,
        }
    }

    pub fn into_protocol(self) -> protocol::Error {
        protocol::Error {
            code: self.code.to_owned(),
            message: self.message,
            retryable: self.retryable,
        }
    }
}

impl From<Failure> for ReadError {
    fn from(failure: Failure) -> ReadError {
        match failure {
            Failure::Exception { .. } => {
                ReadError::new(codes::PRIVATE_API_EXCEPTION, failure.to_string(), false)
            }
            other => ReadError::new(protocol::code::INTERNAL, other.to_string(), false),
        }
    }
}

/// The attribute numbers and the NSNumber keys the snapshot's attribute
/// dictionaries use for them.
pub struct Attributes {
    pub names: Vec<&'static str>,
    pub numbers: Vec<u64>,
    /// The numbers as the `attributes` option.
    pub request: Retained<NSArray<NSNumber>>,
    pub element_type: Option<Retained<NSNumber>>,
    pub base_type: Option<Retained<NSNumber>>,
    pub label: Option<Retained<NSNumber>>,
    pub value: Option<Retained<NSNumber>>,
    pub placeholder: Option<Retained<NSNumber>>,
    pub identifier: Option<Retained<NSNumber>>,
    pub frame: Option<Retained<NSNumber>>,
    pub automation_type: Option<Retained<NSNumber>>,
    pub traits: Option<Retained<NSNumber>>,
    /// The scroll attributes, when the runtime numbers them.
    pub scroll: Option<ScrollAttributes>,
    /// [`INTERACTION_ATTRIBUTE_NAME`]'s key and the request that asks for
    /// it alone, when the runtime numbers it.
    pub interaction: Option<(Retained<NSNumber>, Retained<NSArray<NSNumber>>)>,
}

/// The keys of [`SCROLL_ATTRIBUTE_NAMES`] and the request that asks for
/// them.
pub struct ScrollAttributes {
    pub request: Retained<NSArray<NSNumber>>,
    pub vertical: Retained<NSNumber>,
    pub horizontal: Retained<NSNumber>,
    pub visible_frame: Retained<NSNumber>,
}

/// The keys of a snapshot node dictionary.
pub struct SnapshotKeys {
    pub attributes: Retained<NSString>,
    pub children: Retained<NSString>,
    pub children_count: Retained<NSString>,
    /// The node's own `AXUIElementRef`.
    pub element: Retained<NSString>,
}

pub struct Runtime {
    framework: Retained<AnyObject>,
    element_class: &'static AnyClass,
    ax_element_class: &'static AnyClass,
    settings: Option<Retained<AnyObject>>,
    set_automation: Option<SetBoolFn>,
    automation_enabled: Option<BoolFn>,
    default_parameters: DefaultParametersFn,
    pub(crate) value_get_value: ValueGetValueFn,
    pub(crate) value_type_id: usize,
    pub(crate) dictionary_class: &'static AnyClass,
    pub(crate) array_class: &'static AnyClass,
    pub(crate) attributes: Attributes,
    pub(crate) keys: SnapshotKeys,
}

fn function(name: &CStr) -> Result<*mut c_void, Failure> {
    objc::symbol(name)
}

impl Runtime {
    /// Loads the images, resolves every class and function and creates the
    /// remote-access client. Everything a read needs is ready afterwards.
    pub fn load() -> Result<Runtime, Failure> {
        objc::load(AX_RUNTIME)?;
        objc::load(XCT_AUTOMATION_SUPPORT)?;

        let framework_class = objc::class(c"XCTAccessibilityFramework")?;
        let element_class = objc::class(c"XCAccessibilityElement")?;
        let ax_element_class = objc::class(c"AXElement")?;
        // SAFETY: the signatures are those of agent-device's typedefs, which
        // match the runtime's headers as class-dumped by idb.
        let default_parameters: DefaultParametersFn =
            unsafe { std::mem::transmute(function(c"XCTDefaultSnapshotParameters")?) };
        let attributes_for_names: AttributesForNamesFn = unsafe {
            std::mem::transmute(function(c"XCAXAccessibilityAttributesForStringAttributes")?)
        };
        let value_get_value: ValueGetValueFn =
            unsafe { std::mem::transmute(function(c"AXValueGetValue")?) };
        let value_type_id: TypeIdFn =
            unsafe { std::mem::transmute(function(c"AXValueGetTypeID")?) };
        let automation_enabled: Option<BoolFn> = function(c"_AXSAutomationEnabled")
            .ok()
            // SAFETY: `_AXSAutomationEnabled(void) -> Boolean`.
            .map(|address| unsafe { std::mem::transmute::<*mut c_void, BoolFn>(address) });

        // AXSettings lives in AccessibilityUtilities, which XCTAutomationSupport
        // loads on iOS 26.5; the explicit load and libAccessibility's setter
        // cover a runtime where it does not.
        let mut settings_class = AnyClass::get(c"AXSettings");
        if settings_class.is_none() {
            objc::load(ACCESSIBILITY_UTILITIES)?;
            settings_class = AnyClass::get(c"AXSettings");
        }
        let settings: Option<Retained<AnyObject>> = match settings_class {
            Some(class) => guarded("+[AXSettings sharedInstance]", || unsafe {
                msg_send![class, sharedInstance]
            })?,
            None => None,
        };
        let set_automation: Option<SetBoolFn> = if settings.is_some() {
            None
        } else {
            let setter = function(c"_AXSSetAutomationEnabled")?;
            // SAFETY: `_AXSSetAutomationEnabled(Boolean)`.
            Some(unsafe { std::mem::transmute::<*mut c_void, SetBoolFn>(setter) })
        };

        const INIT: &str = "-[XCTAccessibilityFramework initForRemoteAccess]";
        let framework: Option<Retained<AnyObject>> = guarded(INIT, || unsafe {
            msg_send![msg_send![framework_class, alloc], initForRemoteAccess]
        })?;
        let framework = framework.ok_or(Failure::Nil { call: INIT.into() })?;
        let responds: bool = guarded("-respondsToSelector:", || unsafe {
            msg_send![&*framework, respondsToSelector: sel!(userTestingSnapshotForElement:options:error:)]
        })?;
        if !responds {
            return Err(Failure::Missing(
                "XCTAccessibilityFramework has no userTestingSnapshotForElement:options:error:"
                    .into(),
            ));
        }
        let attributes = resolve_attributes(attributes_for_names)?;

        Ok(Runtime {
            framework,
            element_class,
            ax_element_class,
            settings,
            set_automation,
            automation_enabled,
            default_parameters,
            value_get_value,
            // SAFETY: a C function with no arguments returning a CFTypeID.
            value_type_id: unsafe { value_type_id() },
            dictionary_class: objc::class(c"NSDictionary")?,
            array_class: objc::class(c"NSArray")?,
            attributes,
            keys: SnapshotKeys {
                attributes: NSString::from_str("UIAccessibilitySnapshotKeyAttributes"),
                children: NSString::from_str("UIAccessibilitySnapshotKeyChildren"),
                children_count: NSString::from_str("UIAccessibilitySnapshotKeyChildrenCount"),
                element: NSString::from_str("UIAccessibilitySnapshotKeyElement"),
            },
        })
    }

    /// Whether automation mode is on, without asserting it.
    pub fn automation_state(&self) -> Option<bool> {
        // SAFETY: `_AXSAutomationEnabled(void) -> Boolean`.
        self.automation_enabled
            .map(|enabled| unsafe { enabled() != 0 })
    }

    /// Asserts automation mode, as agent-device does before every read.
    fn assert_automation(&self) {
        if let Some(settings) = &self.settings {
            let _ = guarded("-[AXSettings setAutomationEnabled:]", || unsafe {
                let _: () = msg_send![&**settings, setAutomationEnabled: true];
            });
        } else if let Some(set) = self.set_automation {
            // SAFETY: `_AXSSetAutomationEnabled(Boolean)`.
            unsafe { set(1) };
        }
    }

    /// `+[AXElement primaryApp].pid`: the process that owns the foreground.
    fn primary_pid(&self) -> Result<Option<i32>, Failure> {
        guarded("+[AXElement primaryApp]", || unsafe {
            let app: Option<Retained<AnyObject>> = msg_send![self.ax_element_class, primaryApp];
            app.map(|app| {
                let pid: i32 = msg_send![&*app, pid];
                pid
            })
        })
    }

    /// The application element for `pid` and the `AXUIElementRef` inside
    /// it, which the snapshot call takes. The element owns the ref, so it is
    /// returned to keep it alive.
    fn application_element(
        &self,
        pid: i32,
    ) -> Result<(Retained<AnyObject>, AXUIElementRef), ReadError> {
        let element: Option<Retained<AnyObject>> = guarded(
            "+[XCAccessibilityElement elementWithProcessIdentifier:]",
            || unsafe { msg_send![self.element_class, elementWithProcessIdentifier: pid] },
        )?;
        let missing = || {
            ReadError::new(
                codes::SNAPSHOT_FAILED,
                format!("the accessibility runtime has no element for process {pid}"),
                true,
            )
        };
        let element = element.ok_or_else(missing)?;
        let raw: AXUIElementRef = guarded("-[XCAccessibilityElement AXUIElement]", || unsafe {
            msg_send![&*element, AXUIElement]
        })?;
        if raw.is_null() {
            return Err(missing());
        }
        Ok((element, raw))
    }

    /// One `userTestingSnapshotForElement:options:error:` call for
    /// `attributes`.
    fn snapshot_call(
        &self,
        element: AXUIElementRef,
        attributes: &NSArray<NSNumber>,
        max_depth: u32,
        max_nodes: u32,
    ) -> Result<Result<Retained<AnyObject>, Retained<NSError>>, Failure> {
        guarded(
            "-[XCTAccessibilityFramework userTestingSnapshotForElement:options:error:]",
            || unsafe {
                let defaults = (self.default_parameters)();
                let options: Retained<AnyObject> = if defaults.is_null() {
                    msg_send![
                        objc::class(c"NSMutableDictionary").expect("Foundation is loaded"),
                        new
                    ]
                } else {
                    msg_send![defaults, mutableCopy]
                };
                let set = |key: &str, value: &AnyObject| {
                    let key = NSString::from_str(key);
                    let _: () = msg_send![&*options, setObject: value, forKey: &*key];
                };
                set("attributes", attributes);
                set("maxDepth", &NSNumber::new_u32(max_depth));
                set("maxChildren", &NSNumber::new_u32(max_nodes));
                set("maxArrayCount", &NSNumber::new_u32(max_nodes));
                let mut error: *mut NSError = std::ptr::null_mut();
                let tree: *mut AnyObject = msg_send![
                    &*self.framework,
                    userTestingSnapshotForElement: element,
                    options: &*options,
                    error: &mut error
                ];
                match Retained::retain(tree) {
                    Some(tree) => Ok(tree),
                    None => Err(Retained::retain(error).unwrap_or_else(|| {
                        NSError::new(-1, &NSString::from_str("the snapshot returned nil"))
                    })),
                }
            },
        )
    }

    /// A snapshot of `element` alone with the scroll attributes; `None`
    /// when the runtime numbers no scroll attributes or the call fails, as
    /// it does once the element is gone.
    pub(crate) fn scroll_snapshot(
        &self,
        element: AXUIElementRef,
    ) -> Result<Option<Retained<AnyObject>>, Failure> {
        let Some(scroll) = &self.attributes.scroll else {
            return Ok(None);
        };
        Ok(self.snapshot_call(element, &scroll.request, 0, 1)?.ok())
    }

    /// A snapshot of `element` alone with [`INTERACTION_ATTRIBUTE_NAME`];
    /// `None` when the runtime does not number it or the call fails.
    pub(crate) fn interaction_snapshot(
        &self,
        element: AXUIElementRef,
    ) -> Result<Option<Retained<AnyObject>>, Failure> {
        let Some((_, request)) = &self.attributes.interaction else {
            return Ok(None);
        };
        Ok(self.snapshot_call(element, request, 0, 1)?.ok())
    }

    /// Reads the tree of the process that owns the foreground: an
    /// application, or SpringBoard while it shows the home screen or a system
    /// alert. The owner is checked before and after, and a depth the server
    /// rejects is retried at half depth.
    pub fn read(&self, limits: Limits, scale: f64) -> Result<Read, ReadError> {
        self.assert_automation();
        let Some(pid) = self.primary_pid()? else {
            return Err(ReadError::new(
                codes::NO_FOREGROUND,
                "no application owns the foreground: accessibility is starting, or an application is launching",
                true,
            ));
        };
        let owner: Owner = process::owner(pid);
        let (_element, raw) = self.application_element(pid)?;

        // One level past the limit, so the conversion sees what the limit cuts.
        let wanted = limits.max_depth.saturating_add(1);
        let mut depth = wanted;
        let mut rejected = 0;
        let snapshot = loop {
            match self.snapshot_call(raw, &self.attributes.request, depth, limits.max_nodes)? {
                Ok(tree) => break tree,
                Err(error) => {
                    let (domain, code, ax_error) = describe_error(&error);
                    let rejection = ax_error == Some(AX_ERROR_REJECTED)
                        || (domain == XCTAS_ERROR_DOMAIN && code == 5);
                    if rejection && depth > 1 && rejected < 2 {
                        rejected += 1;
                        depth = (depth / 2).max(1);
                        continue;
                    }
                    let not_responding = ax_error == Some(AX_ERROR_CANNOT_COMPLETE);
                    return Err(ReadError::new(
                        codes::SNAPSHOT_FAILED,
                        format!(
                            "the snapshot of {} (pid {pid}) failed{}: {domain} {code}, AX error {}: {}",
                            owner.bundle_id.as_deref().unwrap_or(&owner.name),
                            if not_responding {
                                ", the application did not answer"
                            } else {
                                ""
                            },
                            ax_error.map_or("none".to_owned(), |e| e.to_string()),
                            error.localizedDescription()
                        ),
                        true,
                    ));
                }
            }
        };

        let after = self.primary_pid()?;
        if after != Some(pid) {
            return Err(ReadError::new(
                protocol::code::WINDOW_CHANGED,
                format!(
                    "the foreground changed from pid {pid} to {} during the read",
                    after.map_or("none".to_owned(), |p| p.to_string())
                ),
                true,
            ));
        }
        let root = extract::element(self, &snapshot, limits.max_nodes)?;
        Ok(Read {
            owner,
            root,
            scale,
            limits,
            depth_cut: depth < wanted,
        })
    }
}

fn resolve_attributes(attributes_for_names: AttributesForNamesFn) -> Result<Attributes, Failure> {
    let numbers_for = |names: &[&'static str]| -> Result<Option<Vec<u64>>, Failure> {
        let strings: Vec<Retained<NSString>> =
            names.iter().map(|name| NSString::from_str(name)).collect();
        let array = NSArray::from_retained_slice(&strings);
        let numbers = guarded(
            "XCAXAccessibilityAttributesForStringAttributes",
            || unsafe {
                let raw = attributes_for_names(Retained::as_ptr(&array).cast_mut().cast());
                Retained::<NSArray<NSNumber>>::retain(raw.cast())
            },
        )?;
        Ok(numbers
            .filter(|numbers| numbers.count() == names.len())
            .map(|numbers| numbers.iter().map(|number| number.as_u64()).collect()))
    };
    let mut names: Vec<&'static str> = ATTRIBUTE_NAMES.to_vec();
    let numbers = match numbers_for(&names)? {
        Some(numbers) => numbers,
        None => {
            names.retain(|name| *name != "XC_kAXXCAttributePlaceholderValue");
            numbers_for(&names)?.ok_or_else(|| {
                Failure::Missing("the runtime does not number the accessibility attributes".into())
            })?
        }
    };
    let key = |name: &str| {
        names
            .iter()
            .position(|candidate| *candidate == name)
            .map(|index| NSNumber::new_u64(numbers[index]))
    };
    let request: Vec<Retained<NSNumber>> = numbers.iter().map(|n| NSNumber::new_u64(*n)).collect();
    let scroll = numbers_for(&SCROLL_ATTRIBUTE_NAMES)?.map(|numbers| {
        let keys: Vec<Retained<NSNumber>> = numbers.iter().map(|n| NSNumber::new_u64(*n)).collect();
        ScrollAttributes {
            vertical: keys[0].clone(),
            horizontal: keys[1].clone(),
            visible_frame: keys[2].clone(),
            request: NSArray::from_retained_slice(&keys),
        }
    });
    let interaction = numbers_for(&[INTERACTION_ATTRIBUTE_NAME])?.map(|numbers| {
        let key = NSNumber::new_u64(numbers[0]);
        let request = NSArray::from_retained_slice(std::slice::from_ref(&key));
        (key, request)
    });
    Ok(Attributes {
        element_type: key("XC_kAXXCAttributeElementType"),
        base_type: key("XC_kAXXCAttributeElementBaseType"),
        label: key("XC_kAXXCAttributeLabel"),
        value: key("XC_kAXXCAttributeValue"),
        placeholder: key("XC_kAXXCAttributePlaceholderValue"),
        identifier: key("XC_kAXXCAttributeIdentifier"),
        frame: key("XC_kAXXCAttributeFrame"),
        automation_type: key("XC_kAXXCAttributeAutomationType"),
        traits: key("XC_kAXXCAttributeTraits"),
        scroll,
        interaction,
        request: NSArray::from_retained_slice(&request),
        names,
        numbers,
    })
}

/// Domain, code and the AX server's own error number from an `NSError`.
fn describe_error(error: &NSError) -> (String, isize, Option<i64>) {
    let key = NSString::from_str("accessibility-error");
    let ax_error = guarded("-[NSError userInfo]", || unsafe {
        let info: Option<Retained<AnyObject>> = msg_send![error, userInfo];
        info.and_then(|info| {
            let value: Option<Retained<AnyObject>> = msg_send![&*info, objectForKey: &*key];
            value.and_then(|value| value.downcast_ref::<NSNumber>().map(|n| n.as_i64()))
        })
    })
    .ok()
    .flatten();
    (error.domain().to_string(), error.code(), ax_error)
}
