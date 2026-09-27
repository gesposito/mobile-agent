//! The host translator: the tree read when the reader cannot run (design
//! D6, spike S4). `AXPTranslator` from AccessibilityPlatformTranslation turns
//! each attribute of an element into a request that its token delegate
//! answers by sending it to the device with
//! `-[SimDevice sendAccessibilityRequestAsync:completionQueue:completionHandler:]`.
//! Nothing runs inside the Simulator, but it takes about seven requests per
//! element, flattens containers and misses what S4 lists, so its reads are
//! degraded.
//!
//! The translator is a process-wide singleton with one delegate slot and no
//! locking of its own, so reads of every device take turns, each pointing
//! the delegate at its device first.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

use block2::RcBlock;
use dispatch2::{DispatchQueue, DispatchQueueAttr, DispatchRetained};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{AllocAnyThread, DefinedClass, define_class, msg_send};
use objc2_foundation::{NSArray, NSNumber, NSRect, NSString, NSUUID};
use protocol::snapshot::Snapshot;
use simulator_reader::elements::{self, Element, Limits, Read, traits, types};
use simulator_reader::guest::process;

use crate::error::{Error, Result, TreeFailure};
use crate::macos::coresim::Device;
use crate::macos::objc::{self, guarded};
use crate::tree::TreeOptions;

pub const ACCESSIBILITY_PLATFORM_TRANSLATION: &str = "/System/Library/PrivateFrameworks/AccessibilityPlatformTranslation.framework/AccessibilityPlatformTranslation";

/// How long one request may wait for the device.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

pub struct DelegateIvars {
    /// The device requests go to: the one being read.
    device: Mutex<Option<Device>>,
    queue: DispatchRetained<DispatchQueue>,
    requests: Arc<AtomicUsize>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements, and the class has no
    // Drop implementation.
    #[unsafe(super(NSObject))]
    #[name = "MobileAgentTranslationTokenDelegate"]
    #[ivars = DelegateIvars]
    pub struct TokenDelegate;

    impl TokenDelegate {
        /// `AXPTranslationTokenDelegateHelper`: the block the translator
        /// calls for every request it cannot answer itself.
        #[unsafe(method_id(accessibilityTranslationDelegateBridgeCallbackWithToken:))]
        fn bridge_callback(&self, _token: &NSString) -> Retained<AnyObject> {
            let ivars = self.ivars();
            let device = lock(&ivars.device).clone();
            let queue = ivars.queue.clone();
            let requests = Arc::clone(&ivars.requests);
            let block = RcBlock::new(move |request: *mut AnyObject| -> *mut AnyObject {
                requests.fetch_add(1, Ordering::Relaxed);
                // SAFETY: the translator passes a live AXPTranslatorRequest.
                let response = unsafe { request.as_ref() }.zip(device.as_ref()).and_then(
                    |(request, device)| {
                        device
                            .send_accessibility_request(request, &queue, REQUEST_TIMEOUT)
                            .ok()
                            .flatten()
                    },
                );
                match response {
                    Some(response) => Retained::autorelease_return(response),
                    None => empty_response(),
                }
            });
            // SAFETY: a heap block is an Objective-C object; into_raw hands
            // over the +1 reference that from_raw takes.
            unsafe { Retained::from_raw(RcBlock::into_raw(block).cast::<AnyObject>()) }
                .expect("a block pointer is not null")
        }

        #[unsafe(method(accessibilityTranslationConvertPlatformFrameToSystem:withToken:))]
        fn convert_frame(&self, rect: NSRect, _token: &NSString) -> NSRect {
            rect
        }

        #[unsafe(method(accessibilityTranslationRootParentWithToken:))]
        fn root_parent(&self, _token: &NSString) -> *mut AnyObject {
            std::ptr::null_mut()
        }
    }
);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// `+[AXPTranslatorResponse emptyResponse]`, autoreleased, for a request the
/// device did not answer.
fn empty_response() -> *mut AnyObject {
    let Ok(class) = objc::class(c"AXPTranslatorResponse") else {
        return std::ptr::null_mut();
    };
    guarded("+[AXPTranslatorResponse emptyResponse]", || {
        // SAFETY: a class method returning a response object.
        let response: Option<Retained<AnyObject>> = unsafe { msg_send![class, emptyResponse] };
        response.map_or(std::ptr::null_mut(), Retained::autorelease_return)
    })
    .unwrap_or(std::ptr::null_mut())
}

/// The translator and the delegate installed in it.
struct Translator {
    translator: Retained<AnyObject>,
    delegate: Retained<TokenDelegate>,
}

// SAFETY: both objects are used only while `TRANSLATOR`'s lock is held, by
// one thread at a time; the delegate's own state is behind a mutex.
unsafe impl Send for Translator {}

static TRANSLATOR: Mutex<Option<Translator>> = Mutex::new(None);

fn load() -> Result<()> {
    static LOADED: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    LOADED
        .get_or_init(|| objc::load(ACCESSIBILITY_PLATFORM_TRANSLATION))
        .clone()
        .map_err(|reason| Error::Load {
            what: "AccessibilityPlatformTranslation".into(),
            reason,
        })
}

/// Whether the translator can be used on this host.
pub fn available() -> Result<()> {
    load()?;
    objc::class(c"AXPTranslator").map(|_| ())
}

impl Translator {
    fn new() -> Result<Translator> {
        load()?;
        let class = objc::class(c"AXPTranslator")?;
        const SHARED: &str = "+[AXPTranslator sharedInstance]";
        let translator: Option<Retained<AnyObject>> = guarded(SHARED, || {
            // SAFETY: a class method returning the shared translator.
            unsafe { msg_send![class, sharedInstance] }
        })?;
        let translator = translator.ok_or_else(|| Error::Framework {
            call: SHARED.into(),
            failure: crate::error::Failure::Nil,
        })?;
        let delegate = TokenDelegate::alloc().set_ivars(DelegateIvars {
            device: Mutex::new(None),
            queue: DispatchQueue::new(
                "dev.mobileagent.simulator.translator",
                DispatchQueueAttr::SERIAL,
            ),
            requests: Arc::new(AtomicUsize::new(0)),
        });
        // SAFETY: NSObject's designated initializer.
        let delegate: Retained<TokenDelegate> = unsafe { msg_send![super(delegate), init] };
        guarded("-[AXPTranslator setBridgeTokenDelegate:]", || {
            // SAFETY: the translator holds its delegate weakly; `Translator`
            // keeps it alive for the process's lifetime.
            unsafe {
                let _: () = msg_send![&*translator, setBridgeTokenDelegate: &*delegate];
            }
        })?;
        Ok(Translator {
            translator,
            delegate,
        })
    }
}

fn set_token(translation: &AnyObject, token: &NSString) -> Result<()> {
    guarded("-[AXPTranslationObject setBridgeDelegateToken:]", || {
        // SAFETY: takes the token string.
        unsafe {
            let _: () = msg_send![translation, setBridgeDelegateToken: token];
        }
    })
}

fn translation_pid(translation: &AnyObject) -> Result<i32> {
    guarded("-[AXPTranslationObject pid]", || {
        // SAFETY: returns the owning process's pid.
        let pid: i32 = unsafe { msg_send![translation, pid] };
        pid
    })
}

/// Reads the tree of the device's frontmost application, or of system UI
/// when it is in front.
pub fn read(device: &Device, scale: f64, options: &TreeOptions) -> Result<Snapshot> {
    let mut slot = lock(&TRANSLATOR);
    if slot.is_none() {
        *slot = Some(Translator::new()?);
    }
    let translator = slot.as_ref().expect("set above");
    *lock(&translator.delegate.ivars().device) = Some(device.clone());
    let result = walk_frontmost(translator, scale, options);
    *lock(&translator.delegate.ivars().device) = None;
    result
}

fn walk_frontmost(translator: &Translator, scale: f64, options: &TreeOptions) -> Result<Snapshot> {
    let token = NSUUID::new().UUIDString();
    const FRONTMOST: &str =
        "-[AXPTranslator frontmostApplicationWithDisplayId:bridgeDelegateToken:]";
    let translation: Option<Retained<AnyObject>> = guarded(FRONTMOST, || {
        // SAFETY: display 0 is the main display; returns a translation or nil.
        unsafe {
            msg_send![
                &*translator.translator,
                frontmostApplicationWithDisplayId: 0u32,
                bridgeDelegateToken: &*token
            ]
        }
    })?;
    let Some(translation) = translation else {
        return Err(Error::Tree {
            kind: TreeFailure::NoForeground,
            message: "the translator names no frontmost application yet".into(),
        });
    };
    set_token(&translation, &token)?;
    let pid = translation_pid(&translation)?;
    const ELEMENT: &str = "-[AXPTranslator macPlatformElementFromTranslation:]";
    let element: Option<Retained<AnyObject>> = guarded(ELEMENT, || {
        // SAFETY: takes a translation, returns its platform element or nil.
        unsafe {
            msg_send![&*translator.translator, macPlatformElementFromTranslation: &*translation]
        }
    })?;
    let element = element.ok_or_else(|| Error::Framework {
        call: ELEMENT.into(),
        failure: crate::error::Failure::Nil,
    })?;
    let limits = Limits {
        max_depth: options.max_depth,
        max_nodes: options.max_nodes,
    };
    let mut walk = Walk {
        token: &token,
        limits,
        count: 0,
        ancestors: Vec::new(),
    };
    let root = walk.element(&element, 0)?;
    Ok(elements::snapshot(&Read {
        owner: process::owner(pid),
        root,
        scale,
        limits,
        depth_cut: false,
    }))
}

struct Walk<'a> {
    token: &'a NSString,
    limits: Limits,
    count: u32,
    ancestors: Vec<*const AnyObject>,
}

fn text(value: Option<Retained<AnyObject>>) -> Option<String> {
    let value = value?;
    if let Some(string) = value.downcast_ref::<NSString>() {
        return Some(string.to_string());
    }
    value
        .downcast_ref::<NSNumber>()
        .map(|number| number.stringValue().to_string())
}

impl Walk<'_> {
    fn element(&mut self, element: &AnyObject, level: u32) -> Result<Element> {
        self.count += 1;
        // Each element resolves its attributes through its own translation,
        // which must carry the read's token.
        let translation: Option<Retained<AnyObject>> =
            guarded("-[AXPMacPlatformElement translation]", || {
                // SAFETY: returns the element's translation object or nil.
                unsafe { msg_send![element, translation] }
            })?;
        if let Some(translation) = &translation {
            set_token(translation, self.token)?;
        }
        let read = |call: &str, body: &dyn Fn() -> Option<Retained<AnyObject>>| {
            guarded(call, body).map(text)
        };
        // SAFETY (the reads below): NSAccessibility getters returning an
        // object or nil.
        let role = read("-accessibilityRole", &|| unsafe {
            msg_send![element, accessibilityRole]
        })?;
        let subrole = read("-accessibilitySubrole", &|| unsafe {
            msg_send![element, accessibilitySubrole]
        })?;
        let label = read("-accessibilityLabel", &|| unsafe {
            msg_send![element, accessibilityLabel]
        })?;
        let identifier = read("-accessibilityIdentifier", &|| unsafe {
            msg_send![element, accessibilityIdentifier]
        })?;
        let value = read("-accessibilityValue", &|| unsafe {
            msg_send![element, accessibilityValue]
        })?;
        let frame: NSRect = guarded("-accessibilityFrame", || {
            // SAFETY: returns the element's frame.
            unsafe { msg_send![element, accessibilityFrame] }
        })?;
        let key = NSString::from_str("AXTraits");
        let raw_traits: Option<Retained<AnyObject>> =
            guarded("-accessibilityAttributeValue: AXTraits", || {
                // SAFETY: takes an attribute name, returns its value or nil.
                unsafe { msg_send![element, accessibilityAttributeValue: &*key] }
            })?;
        let mut element_traits = raw_traits
            .and_then(|value| value.downcast_ref::<NSNumber>().map(NSNumber::as_u64))
            .unwrap_or(0);
        let automation_type = automation_type(role.as_deref(), subrole.as_deref());
        if role.as_deref() == Some("AXHeading") {
            element_traits |= traits::HEADER;
        }
        let text_input = matches!(
            automation_type,
            types::TEXT_FIELD | types::SECURE_TEXT_FIELD | types::SEARCH_FIELD | types::TEXT_VIEW
        );
        let placeholder = if text_input
            && element
                .class()
                .responds_to(objc2::sel!(accessibilityPlaceholderValue))
        {
            read("-accessibilityPlaceholderValue", &|| unsafe {
                msg_send![element, accessibilityPlaceholderValue]
            })?
        } else {
            None
        };
        let mut node = Element {
            automation_type: Some(automation_type),
            class: None,
            label,
            identifier,
            value,
            placeholder,
            traits: element_traits,
            frame: Some([
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            ]),
            children_count: None,
            children: Vec::new(),
            // AXPTranslator has no attribute for a scroll view's viewport.
            scroll_area: None,
            user_interaction: None,
        };
        if level >= self.limits.max_depth {
            return Ok(node);
        }
        let children: Option<Retained<NSArray<AnyObject>>> =
            guarded("-accessibilityChildren", || {
                // SAFETY: returns an array of elements or nil.
                unsafe { msg_send![element, accessibilityChildren] }
            })?;
        let Some(children) = children else {
            return Ok(node);
        };
        node.children_count = Some(children.count() as u64);
        self.ancestors.push(element as *const AnyObject);
        for child in children.iter() {
            if self.count >= self.limits.max_nodes {
                break;
            }
            // An element that lists an ancestor as its child would loop.
            if self.ancestors.contains(&(&*child as *const AnyObject)) {
                continue;
            }
            let child = objc2::rc::autoreleasepool(|_| self.element(&child, level + 1))?;
            node.children.push(child);
        }
        self.ancestors.pop();
        Ok(node)
    }
}

/// The `XCUIElementType` value for an AX role and subrole, so both read
/// paths name classes alike.
pub fn automation_type(role: Option<&str>, subrole: Option<&str>) -> u64 {
    let named = |name| elements::xcui_element_type(name).unwrap_or(types::OTHER);
    match (role.unwrap_or_default(), subrole.unwrap_or_default()) {
        ("AXWindow", _) => types::RUNTIME_WINDOW,
        ("AXButton", _) | ("AXRadioButton", "AXTabButton") => named("Button"),
        ("AXRadioButton", _) => named("RadioButton"),
        ("AXCheckBox", "AXSwitch") | ("AXSwitch", _) => types::SWITCH,
        ("AXCheckBox", _) => named("CheckBox"),
        ("AXStaticText" | "AXHeading", _) => types::STATIC_TEXT,
        ("AXTextField", "AXSecureTextField") => types::SECURE_TEXT_FIELD,
        ("AXTextField", "AXSearchField") | ("AXSearchField", _) => types::SEARCH_FIELD,
        ("AXTextField", _) => types::TEXT_FIELD,
        ("AXTextArea", _) => types::TEXT_VIEW,
        ("AXImage", _) => named("Image"),
        ("AXLink", _) => named("Link"),
        ("AXSlider", _) => named("Slider"),
        ("AXPopUpButton", _) => named("PopUpButton"),
        ("AXComboBox", _) => named("ComboBox"),
        ("AXTabGroup", _) => named("TabGroup"),
        ("AXToolbar", _) => named("Toolbar"),
        ("AXScrollArea", _) => types::SCROLL_VIEW,
        ("AXTable" | "AXList" | "AXOutline", _) => types::TABLE,
        ("AXCell" | "AXRow", _) => named("Cell"),
        ("AXProgressIndicator", _) => named("ProgressIndicator"),
        ("AXBusyIndicator", _) => named("ActivityIndicator"),
        ("AXMenu", _) => named("Menu"),
        ("AXMenuItem", _) => named("MenuItem"),
        ("AXWebArea", _) => named("WebView"),
        _ => types::OTHER,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_map_to_the_types_the_reader_reports() {
        assert_eq!(automation_type(Some("AXButton"), None), 9);
        assert_eq!(
            automation_type(Some("AXRadioButton"), Some("AXTabButton")),
            9
        );
        assert_eq!(
            automation_type(Some("AXTextField"), Some("AXSecureTextField")),
            types::SECURE_TEXT_FIELD
        );
        assert_eq!(
            automation_type(Some("AXTextField"), None),
            types::TEXT_FIELD
        );
        assert_eq!(automation_type(Some("AXHeading"), None), types::STATIC_TEXT);
        assert_eq!(automation_type(Some("AXGroup"), None), types::OTHER);
        assert_eq!(automation_type(None, None), types::OTHER);
        let heading = Element {
            automation_type: Some(automation_type(Some("AXHeading"), None)),
            traits: traits::HEADER,
            ..Element::default()
        };
        assert_eq!(elements::class_name(2, &heading), "XCUIElementTypeHeading");
    }

    #[test]
    fn the_translator_framework_loads_and_names_its_classes() {
        load().expect("AccessibilityPlatformTranslation loads");
        for name in [
            c"AXPTranslator",
            c"AXPTranslatorResponse",
            c"AXPMacPlatformElement",
        ] {
            objc::class(name).expect("the class is registered");
        }
    }
}
