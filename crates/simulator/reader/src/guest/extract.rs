//! Turns the snapshot the runtime returns into [`Element`]s.
//!
//! A snapshot node is an `NSDictionary` with the attribute dictionary under
//! `UIAccessibilitySnapshotKeyAttributes` (keyed by attribute number), the
//! children under `UIAccessibilitySnapshotKeyChildren` and the child count
//! the server knew of under `UIAccessibilitySnapshotKeyChildrenCount`.
//! Values are `NSString`, `NSNumber`, or an `AXValueRef` for frames.
//!
//! A scroll view on screen is asked for its scroll area in a snapshot of
//! its own element (`UIAccessibilitySnapshotKeyElement`), since asking every
//! node for it costs a read far more (see
//! [`SCROLL_ATTRIBUTE_NAMES`](crate::guest::runtime::SCROLL_ATTRIBUTE_NAMES)).

use std::ffi::c_void;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSNumber, NSString};

use crate::elements::{self, Element, ScrollArea};
use crate::guest::objc::{AXUIElementRef, guarded};
use crate::guest::runtime::{ReadError, Runtime};

/// `kAXValueCGPointType` in AXRuntime's `AXValueType`.
const AX_VALUE_CG_POINT: u32 = 1;
/// `kAXValueCGRectType` in AXRuntime's `AXValueType`.
const AX_VALUE_CG_RECT: u32 = 3;

/// The most scroll views asked for their scroll area in one read; each is
/// a round trip to the application.
pub const MAX_SCROLL_VIEWS: usize = 32;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(cf: *const c_void) -> usize;
}

fn malformed(message: &str) -> ReadError {
    ReadError::new(
        protocol::code::INTERNAL,
        format!("the runtime returned a malformed snapshot: {message}"),
        false,
    )
}

/// The element tree of `snapshot`, with at most one node past `max_nodes`
/// so the conversion sees the limit cut it.
pub fn element(
    runtime: &Runtime,
    snapshot: &AnyObject,
    max_nodes: u32,
) -> Result<Element, ReadError> {
    let mut extractor = Extractor {
        runtime,
        budget: max_nodes as usize + 1,
        screen: None,
        scroll_views: MAX_SCROLL_VIEWS,
    };
    guarded("reading the snapshot dictionaries", || {
        extractor.node(snapshot, 0)
    })?
    .and_then(|root| root.ok_or_else(|| malformed("it has no root node")))
}

struct Extractor<'a> {
    runtime: &'a Runtime,
    /// Nodes still to read.
    budget: usize,
    /// The application element's frame, which scroll views must overlap to
    /// be asked for their scroll area.
    screen: Option<[f64; 4]>,
    /// Scroll views still to ask; none after one fails.
    scroll_views: usize,
}

fn is_kind(object: &AnyObject, class: &AnyClass) -> bool {
    // SAFETY: `isKindOfClass:` takes a class and returns BOOL.
    unsafe { msg_send![object, isKindOfClass: class] }
}

fn object_for(dictionary: &AnyObject, key: &AnyObject) -> Option<Retained<AnyObject>> {
    // SAFETY: `dictionary` is an NSDictionary; `objectForKey:` returns an
    // object or nil.
    unsafe { msg_send![dictionary, objectForKey: key] }
}

fn array_items(array: &AnyObject) -> Vec<Retained<AnyObject>> {
    // SAFETY: `array` is an NSArray; indexes stay below its count.
    unsafe {
        let count: usize = msg_send![array, count];
        (0..count)
            .filter_map(|index| {
                let item: Option<Retained<AnyObject>> = msg_send![array, objectAtIndex: index];
                item
            })
            .collect()
    }
}

fn text(value: &AnyObject) -> Option<String> {
    if let Some(string) = value.downcast_ref::<NSString>() {
        return Some(string.to_string());
    }
    value
        .downcast_ref::<NSNumber>()
        .map(|number| number.stringValue().to_string())
}

fn number(value: &AnyObject) -> Option<u64> {
    value.downcast_ref::<NSNumber>().map(NSNumber::as_u64)
}

impl Extractor<'_> {
    fn node(&mut self, snapshot: &AnyObject, level: u32) -> Result<Option<Element>, ReadError> {
        let runtime = self.runtime;
        if !is_kind(snapshot, runtime.dictionary_class) {
            return Err(malformed("a node is not a dictionary"));
        }
        if self.budget == 0 {
            return Ok(None);
        }
        self.budget -= 1;

        let keys = &runtime.keys;
        let attributes = object_for(snapshot, &keys.attributes)
            .filter(|attributes| is_kind(attributes, runtime.dictionary_class))
            .ok_or_else(|| malformed("a node has no attribute dictionary"))?;
        let names = &runtime.attributes;
        let get = |key: &Option<Retained<NSNumber>>| {
            key.as_ref().and_then(|key| object_for(&attributes, key))
        };
        let mut element = Element {
            automation_type: get(&names.automation_type).and_then(|v| number(&v)),
            class: get(&names.element_type)
                .and_then(|v| text(&v))
                .or_else(|| get(&names.base_type).and_then(|v| text(&v))),
            label: get(&names.label).and_then(|v| text(&v)),
            identifier: get(&names.identifier).and_then(|v| text(&v)),
            value: get(&names.value).and_then(|v| text(&v)),
            placeholder: get(&names.placeholder).and_then(|v| text(&v)),
            traits: get(&names.traits).and_then(|v| number(&v)).unwrap_or(0),
            frame: get(&names.frame).and_then(|v| self.frame(&v)),
            children_count: object_for(snapshot, &keys.children_count).and_then(|v| number(&v)),
            children: Vec::new(),
            scroll_area: None,
        };
        if level == 0 {
            self.screen = element.frame.filter(|f| f[2] > 0.0 && f[3] > 0.0);
        }
        if self.scroll_views > 0
            && elements::is_scroll_view(element.automation_type)
            && elements::on_screen(element.frame, self.screen)
            && let Some(own) = object_for(snapshot, &keys.element)
        {
            element.scroll_area = self.scroll_area(&own)?;
        }
        if let Some(children) = object_for(snapshot, &keys.children)
            .filter(|children| is_kind(children, runtime.array_class))
        {
            for child in array_items(&children) {
                match self.node(&child, level + 1)? {
                    Some(child) => element.children.push(child),
                    None => break,
                }
            }
        }
        Ok(Some(element))
    }

    fn frame(&self, value: &AnyObject) -> Option<[f64; 4]> {
        self.value(value, AX_VALUE_CG_RECT)
    }

    /// The `AXValueRef` in `value` as `N` CGFloats of AXRuntime's type
    /// `kind`: a CGPoint's 2 or a CGRect's 4.
    fn value<const N: usize>(&self, value: &AnyObject, kind: u32) -> Option<[f64; N]> {
        let raw = (value as *const AnyObject).cast::<c_void>();
        // SAFETY: every Objective-C object is a CF type for CFGetTypeID.
        if unsafe { CFGetTypeID(raw) } != self.runtime.value_type_id {
            return None;
        }
        let mut out = [0.0f64; N];
        // SAFETY: `raw` is an AXValueRef and `out` holds as many CGFloats
        // as a value of `kind`.
        let ok = unsafe { (self.runtime.value_get_value)(raw, kind, out.as_mut_ptr().cast()) };
        (ok != 0 && out.iter().all(|v| v.is_finite())).then_some(out)
    }

    /// The scroll area of the scroll view whose element is `own`, from a
    /// snapshot of that element alone. A call that fails, as one does when
    /// the application stops answering, ends the asking for this read.
    fn scroll_area(&mut self, own: &AnyObject) -> Result<Option<ScrollArea>, ReadError> {
        self.scroll_views -= 1;
        let Some(attributes) = &self.runtime.attributes.scroll else {
            return Ok(None);
        };
        let element = AXUIElementRef((own as *const AnyObject).cast_mut().cast());
        let Some(snapshot) = self.runtime.scroll_snapshot(element)? else {
            self.scroll_views = 0;
            return Ok(None);
        };
        let Some(values) = object_for(&snapshot, &self.runtime.keys.attributes)
            .filter(|values| is_kind(values, self.runtime.dictionary_class))
        else {
            return Ok(None);
        };
        let point = |key: &NSNumber| {
            object_for(&values, key).and_then(|v| self.value::<2>(&v, AX_VALUE_CG_POINT))
        };
        Ok(Some(ScrollArea {
            vertical: point(&attributes.vertical),
            horizontal: point(&attributes.horizontal),
            visible_frame: object_for(&values, &attributes.visible_frame)
                .and_then(|v| self.frame(&v)),
        }))
    }
}
