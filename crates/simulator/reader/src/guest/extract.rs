//! Turns the snapshot the runtime returns into [`Element`]s.
//!
//! A snapshot node is an `NSDictionary` with the attribute dictionary under
//! `UIAccessibilitySnapshotKeyAttributes` (keyed by attribute number), the
//! children under `UIAccessibilitySnapshotKeyChildren` and the child count
//! the server knew of under `UIAccessibilitySnapshotKeyChildrenCount`.
//! Values are `NSString`, `NSNumber`, or an `AXValueRef` for frames.

use std::ffi::c_void;

use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_foundation::{NSNumber, NSString};

use crate::elements::Element;
use crate::guest::objc::guarded;
use crate::guest::runtime::{ReadError, Runtime};

/// `kAXValueCGRectType` in AXRuntime's `AXValueType`.
const AX_VALUE_CG_RECT: u32 = 3;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFGetTypeID(cf: *const c_void) -> usize;
}

#[repr(C)]
#[derive(Default)]
struct Rect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
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
    };
    guarded("reading the snapshot dictionaries", || {
        extractor.node(snapshot)
    })?
    .and_then(|root| root.ok_or_else(|| malformed("it has no root node")))
}

struct Extractor<'a> {
    runtime: &'a Runtime,
    /// Nodes still to read.
    budget: usize,
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
    fn node(&mut self, snapshot: &AnyObject) -> Result<Option<Element>, ReadError> {
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
        };
        if let Some(children) = object_for(snapshot, &keys.children)
            .filter(|children| is_kind(children, runtime.array_class))
        {
            for child in array_items(&children) {
                match self.node(&child)? {
                    Some(child) => element.children.push(child),
                    None => break,
                }
            }
        }
        Ok(Some(element))
    }

    fn frame(&self, value: &AnyObject) -> Option<[f64; 4]> {
        let raw = (value as *const AnyObject).cast::<c_void>();
        // SAFETY: every Objective-C object is a CF type for CFGetTypeID.
        if unsafe { CFGetTypeID(raw) } != self.runtime.value_type_id {
            return None;
        }
        let mut rect = Rect::default();
        // SAFETY: `raw` is an AXValueRef and `rect` a CGRect-sized buffer.
        let ok = unsafe {
            (self.runtime.value_get_value)(raw, AX_VALUE_CG_RECT, (&mut rect as *mut Rect).cast())
        };
        let frame = [rect.x, rect.y, rect.width, rect.height];
        (ok != 0 && frame.iter().all(|v| v.is_finite())).then_some(frame)
    }
}
