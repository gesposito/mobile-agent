// The Objective-C side of the runner bundle (design D20).
//
// - MobileAgentRunnerTests, an XCTestCase whose one test starts the Rust agent
//   (agent_ios_start, in the agent's static library, which build.sh links into
//   this bundle) and spins the main run loop forever, the shape of
//   WebDriverAgent's UITestingUITests.
// - At load, XCTest's idle waits are replaced by no-ops, as WebDriverAgent's
//   XCUIApplicationProcess+FBQuiescence does. XCTest 25228 (Xcode 27.0) names
//   them waitForQuiescenceIncludingAnimationsIdle:isPreEvent: and
//   waitForQuiescenceIncludingAnimationsIdle:usingActivity:isPreEvent: on
//   XCUIApplicationProcess and waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:
//   on XCAXClient_iOS; the test logs which it replaced.
// - The C functions below are the XCTest calls the agent makes (src/shim.rs):
//   the foreground's pid and bundle, the snapshot XCTest's accessibility
//   client asks testmanagerd for, as JSON, taps and swipes as an
//   XCSynthesizedEventRecord sent to XCUIDevice's
//   event synthesizer, the hardware buttons, typing through the daemon proxy's
//   _XCT_sendString:maximumFrequency:completion:, setting the focused
//   element's value through its _XCT_setAttribute:value:element:reply:, and
//   XCUIScreen's screenshot.
//
// Selectors and type encodings of the private calls were read with otool -oV
// from Xcode 27.0's iPhoneOS frameworks (docs/spikes/s10-ios-runner.md,
// Private call sites). Every function returns 0 on success; on failure it
// returns another value and sets *err to a malloc'd UTF-8 message the caller
// frees with ma_free. Each runs inside an autorelease pool of its own, since
// the agent calls them from its worker thread.

#import <Foundation/Foundation.h>
#import <UIKit/UIKit.h>
#import <XCTest/XCTest.h>
#import <objc/message.h>
#import <objc/runtime.h>
#include <dlfcn.h>
#include <stdatomic.h>

// ---------------------------------------------------------------------------
// Private declarations.

@interface XCPointerEventPath : NSObject
- (instancetype)initForTouchAtPoint:(CGPoint)point offset:(double)offset;
- (void)moveToPoint:(CGPoint)point atOffset:(double)offset;
- (void)liftUpAtOffset:(double)offset;
@end

@interface XCSynthesizedEventRecord : NSObject
- (instancetype)initWithName:(NSString *)name interfaceOrientation:(long long)orientation;
- (void)addPointerEventPath:(XCPointerEventPath *)path;
@end

@protocol MAEventSynthesizer <NSObject>
- (id)synthesizeEvent:(XCSynthesizedEventRecord *)event completion:(void (^)(BOOL, NSError *))completion;
@end

@interface XCUIDevice (MAPrivate)
- (id<MAEventSynthesizer>)eventSynthesizer;
- (id)accessibilityInterface;
@end

@interface XCAccessibilityElement : NSObject
+ (instancetype)elementWithProcessIdentifier:(int)pid;
- (int)processIdentifier;
@end

@protocol MAAXClient <NSObject>
- (NSDictionary *)defaultParameters;
- (id)requestSnapshotForElement:(XCAccessibilityElement *)element
                     attributes:(NSArray<NSString *> *)attributes
                     parameters:(NSDictionary *)parameters
                          error:(NSError **)error;
@end

// XCTAutomationSupport's names to numbers, which key a snapshot's
// additionalAttributes.
typedef id (*MAAttributesForNames)(id names);

@protocol MADaemonProxy <NSObject>
- (void)_XCT_sendString:(NSString *)string
       maximumFrequency:(NSUInteger)frequency
             completion:(void (^)(NSError *))completion;
- (void)_XCT_setAttribute:(NSNumber *)attribute
                    value:(id)value
                  element:(XCAccessibilityElement *)element
                    reply:(void (^)(BOOL, NSError *))reply;
@end

@interface XCTRunnerDaemonSession : NSObject
+ (instancetype)sharedSession;
- (id<MADaemonProxy>)daemonProxy;
@end

// The agent, in the Rust static library.
extern int agent_ios_start(unsigned short port);

// The loopback port the agent listens on (ios::RUNNER_PORT).
static const unsigned short kAgentPort = 27183;

// The value attribute's number, which the daemon's _XCT_setAttribute takes
// (spike S10; the attribute's name, XC_kAXXCAttributeValue, goes to
// XCAXClient_iOS instead).
static const long long kValueAttribute = 2006;

// ---------------------------------------------------------------------------
// Helpers.

static char *ma_strdup(NSString *s) {
  return strdup(s.UTF8String ?: "");
}

static int ma_fail(char **err, NSString *message) {
  if (err) *err = ma_strdup(message);
  return 1;
}

static NSString *ma_describe(NSError *e) {
  if (!e) return @"(no error)";
  return [NSString stringWithFormat:@"%@ %ld: %@", e.domain, (long)e.code, e.localizedDescription];
}

static NSString *ma_exception(NSException *e) {
  return [NSString stringWithFormat:@"%@: %@", e.name, e.reason];
}

static NSString *ma_json(id object) {
  NSData *json = [NSJSONSerialization dataWithJSONObject:object options:0 error:nil];
  return json ? [[NSString alloc] initWithData:json encoding:NSUTF8StringEncoding] : nil;
}

void ma_free(void *p) { free(p); }

// Runs block on the main thread and returns once it ran: XCUIElement,
// XCUIScreen, XCUIDevice's buttons and UIScreen are main-thread API, and the
// test spins the main run loop for them.
static void ma_on_main(dispatch_block_t block) {
  if ([NSThread isMainThread]) {
    block();
  } else {
    dispatch_sync(dispatch_get_main_queue(), block);
  }
}

// Waits off the main thread for done to be set, at most timeout seconds; on
// the main thread it spins the run loop in 1 ms steps instead, so that a reply
// delivered there can arrive.
static BOOL ma_wait(dispatch_semaphore_t sem, atomic_bool *done, double timeout) {
  if (![NSThread isMainThread]) {
    return dispatch_semaphore_wait(sem, dispatch_time(DISPATCH_TIME_NOW, (int64_t)(timeout * NSEC_PER_SEC))) == 0;
  }
  NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:timeout];
  while (!atomic_load(done)) {
    if ([deadline timeIntervalSinceNow] <= 0) return NO;
    CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.001, true);
  }
  return YES;
}

static id<MADaemonProxy> ma_daemon(void) {
  return [(XCTRunnerDaemonSession *)[NSClassFromString(@"XCTRunnerDaemonSession") sharedSession] daemonProxy];
}

// ---------------------------------------------------------------------------
// Idle waits.

static NSMutableArray<NSString *> *gReplaced;

static void ma_noop_bb(id self, SEL _cmd, BOOL a, BOOL b) {}
static void ma_noop_bib(id self, SEL _cmd, BOOL a, id activity, BOOL b) {}
static void ma_noop_b(id self, SEL _cmd, BOOL a) {}

static void ma_replace(NSString *className, NSString *selector, IMP imp) {
  Class cls = NSClassFromString(className);
  Method m = cls ? class_getInstanceMethod(cls, NSSelectorFromString(selector)) : NULL;
  if (!m) return;
  method_setImplementation(m, imp);
  [gReplaced addObject:[NSString stringWithFormat:@"-[%@ %@]", className, selector]];
}

__attribute__((constructor)) static void ma_replace_idle_waits(void) {
  gReplaced = [NSMutableArray array];
  ma_replace(@"XCUIApplicationProcess", @"waitForQuiescenceIncludingAnimationsIdle:isPreEvent:", (IMP)ma_noop_bb);
  ma_replace(@"XCUIApplicationProcess", @"waitForQuiescenceIncludingAnimationsIdle:usingActivity:isPreEvent:", (IMP)ma_noop_bib);
  ma_replace(@"XCAXClient_iOS", @"waitForQuiescenceOnAllForegroundApplicationsAsPreEvent:", (IMP)ma_noop_b);
}

// ---------------------------------------------------------------------------
// Screen.

static double gScale = 0;

double ma_screen_scale(void) {
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    ma_on_main(^{
      gScale = UIScreen.mainScreen.scale;
    });
  });
  return gScale;
}

// ---------------------------------------------------------------------------
// The foreground.

// The pids of XCAXClient_iOS's active applications, the foreground first as
// XCTest orders them, and of its system application, as JSON:
// {"active": [pid, ...], "system": pid}. WebDriverAgent's fb_activeApplication
// takes the foreground the same way.
char *ma_foreground(void) {
  @autoreleasepool {
    NSMutableArray *active = [NSMutableArray array];
    __block NSNumber *system = @(-1);
    ma_on_main(^{
      @try {
        id client = [XCUIDevice.sharedDevice accessibilityInterface];
        SEL activeSel = NSSelectorFromString(@"activeApplications");
        SEL systemSel = NSSelectorFromString(@"systemApplication");
        if ([client respondsToSelector:activeSel]) {
          for (id app in ((id (*)(id, SEL))objc_msgSend)(client, activeSel)) {
            if ([app respondsToSelector:@selector(processIdentifier)]) [active addObject:@([app processIdentifier])];
          }
        }
        if ([client respondsToSelector:systemSel]) {
          id sys = ((id (*)(id, SEL))objc_msgSend)(client, systemSel);
          if ([sys respondsToSelector:@selector(processIdentifier)]) system = @([sys processIdentifier]);
        }
      } @catch (NSException *e) {
        NSLog(@"mobile-agent runner: the active applications: %@", ma_exception(e));
      }
    });
    return ma_strdup(ma_json(@{@"active" : active, @"system" : system}) ?: @"{}");
  }
}

// The bundle identifier of pid's application, from the daemon session's
// -requestApplicationSpecifierForPID:reply:, since the runner's sandbox hides
// other processes' executables. NULL when the daemon names none within 5 s.
char *ma_bundle_for_pid(int pid) {
  @autoreleasepool {
    id session = [NSClassFromString(@"XCTRunnerDaemonSession") sharedSession];
    SEL sel = NSSelectorFromString(@"requestApplicationSpecifierForPID:reply:");
    if (![session respondsToSelector:sel]) return NULL;
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block atomic_bool done = false;
    __block NSString *bundle = nil;
    void (^reply)(id, NSError *) = ^(id spec, NSError *e) {
      if ([spec respondsToSelector:@selector(bundleIdentifier)]) bundle = [spec performSelector:@selector(bundleIdentifier)];
      atomic_store(&done, true);
      dispatch_semaphore_signal(sem);
    };
    ((void (*)(id, SEL, int, id))objc_msgSend)(session, sel, pid, reply);
    if (!ma_wait(sem, &done, 5)) return NULL;
    return bundle ? ma_strdup(bundle) : NULL;
  }
}

// ---------------------------------------------------------------------------
// The tree.

static NSMutableDictionary<NSString *, XCUIApplication *> *gApps;

// The XCUIApplication of bundle, made once. Main thread.
static XCUIApplication *ma_app(NSString *bundle) {
  if (!gApps) gApps = [NSMutableDictionary dictionary];
  XCUIApplication *app = gApps[bundle];
  if (!app) {
    app = [[XCUIApplication alloc] initWithBundleIdentifier:bundle];
    gApps[bundle] = app;
  }
  return app;
}

// What the tree asks testmanagerd for: the Simulator reader's attributes,
// with XC_kAXXCAttributeVisibleFrame and the scroll areas in the same request
// (asked element by element, a scroll view's took 107 ms), and without
// XC_kAXXCAttributeChildren, whose array the runner's decoder refuses, so that
// the reply never arrives ("Timed out while fetching snapshot from
// testmanagerd" after 65 s; spike S10 and task 4.4).
static NSArray<NSString *> *ma_tree_attributes(void) {
  return @[
    @"XC_kAXXCAttributeElementType", @"XC_kAXXCAttributeElementBaseType", @"XC_kAXXCAttributeAutomationType",
    @"XC_kAXXCAttributeLabel", @"XC_kAXXCAttributeValue", @"XC_kAXXCAttributePlaceholderValue",
    @"XC_kAXXCAttributeIdentifier", @"XC_kAXXCAttributeFrame", @"XC_kAXXCAttributeTraits",
    @"XC_kAXXCAttributeIsUserInteractionEnabled", @"XC_kAXXCAttributeVisibleVerticalScrollArea",
    @"XC_kAXXCAttributeVisibleHorizontalScrollArea", @"XC_kAXXCAttributeVisibleFrame"
  ];
}

// The number of each attribute of the tree, as testmanagerd keys a
// snapshot's additionalAttributes with it; nil without XCTAutomationSupport's
// XCAXAccessibilityAttributesForStringAttributes.
static NSDictionary<NSString *, id> *ma_attribute_numbers(void) {
  static NSDictionary *numbers;
  static dispatch_once_t once;
  dispatch_once(&once, ^{
    MAAttributesForNames convert = (MAAttributesForNames)dlsym(RTLD_DEFAULT, "XCAXAccessibilityAttributesForStringAttributes");
    if (!convert) return;
    NSArray *names = ma_tree_attributes();
    id converted = convert(names);
    NSArray *values = [converted isKindOfClass:NSArray.class] ? converted : nil;
    if (values.count != names.count) return;
    numbers = [NSDictionary dictionaryWithObjects:values forKeys:names];
  });
  return numbers;
}

// A {X, Y} dictionary as [x, y], nil for the X a non-scrolling element has.
static NSArray *ma_point(id value) {
  if (![value isKindOfClass:NSDictionary.class]) return nil;
  NSNumber *x = value[@"X"], *y = value[@"Y"];
  if (![x isKindOfClass:NSNumber.class] || ![y isKindOfClass:NSNumber.class]) return nil;
  if (x.longLongValue == INT_MAX) return nil;
  return @[ x, y ];
}

// A {X, Y, Width, Height} dictionary as [x, y, width, height].
static NSArray *ma_rect(id value) {
  if (![value isKindOfClass:NSDictionary.class]) return nil;
  NSArray *keys = @[ @"X", @"Y", @"Width", @"Height" ];
  NSMutableArray *rect = [NSMutableArray arrayWithCapacity:4];
  for (NSString *key in keys) {
    if (![value[key] isKindOfClass:NSNumber.class]) return nil;
    [rect addObject:value[key]];
  }
  return rect;
}

// One node of the snapshot as src/xctree.rs reads it: {"t": elementType,
// "tr": traits, "f": [x, y, width, height], "i": identifier, "l": label, "v":
// value, "p": placeholder, "kf": true for keyboard focus, "cl": the platform
// class, "ui": whether it takes touches, "sv" and "sh": its scroll areas,
// "vf": its visible frame, "c": [children]}, strings only when present, the
// scroll areas only for an element that scrolls.
static NSDictionary *ma_node(id<XCUIElementSnapshot> snap, NSDictionary<NSString *, id> *numbers) {
  NSMutableDictionary *d = [NSMutableDictionary dictionary];
  d[@"t"] = @(snap.elementType);
  if ([(id)snap respondsToSelector:@selector(traits)]) {
    d[@"tr"] = @(((unsigned long long (*)(id, SEL))objc_msgSend)(snap, @selector(traits)));
  }
  CGRect f = snap.frame;
  d[@"f"] = @[ @(f.origin.x), @(f.origin.y), @(f.size.width), @(f.size.height) ];
  if (snap.identifier.length) d[@"i"] = snap.identifier;
  if (snap.label.length) d[@"l"] = snap.label;
  id v = snap.value;
  if ([v isKindOfClass:NSString.class] && [(NSString *)v length]) d[@"v"] = v;
  else if ([v isKindOfClass:NSNumber.class]) d[@"v"] = [(NSNumber *)v stringValue];
  if (snap.placeholderValue.length) d[@"p"] = snap.placeholderValue;
  if ([(id)snap respondsToSelector:@selector(hasKeyboardFocus)] &&
      ((BOOL (*)(id, SEL))objc_msgSend)(snap, @selector(hasKeyboardFocus))) {
    d[@"kf"] = @YES;
  }
  NSDictionary *extra = [(id)snap respondsToSelector:NSSelectorFromString(@"additionalAttributes")]
                            ? [(id)snap valueForKey:@"additionalAttributes"]
                            : nil;
  if ([extra isKindOfClass:NSDictionary.class] && numbers) {
    id cls = extra[numbers[@"XC_kAXXCAttributeElementType"]] ?: extra[numbers[@"XC_kAXXCAttributeElementBaseType"]];
    if ([cls isKindOfClass:NSString.class] && [(NSString *)cls length]) d[@"cl"] = cls;
    id touches = extra[numbers[@"XC_kAXXCAttributeIsUserInteractionEnabled"]];
    if ([touches isKindOfClass:NSNumber.class]) d[@"ui"] = @([touches boolValue]);
    NSArray *vertical = ma_point(extra[numbers[@"XC_kAXXCAttributeVisibleVerticalScrollArea"]]);
    NSArray *horizontal = ma_point(extra[numbers[@"XC_kAXXCAttributeVisibleHorizontalScrollArea"]]);
    if (vertical || horizontal) {
      if (vertical) d[@"sv"] = vertical;
      if (horizontal) d[@"sh"] = horizontal;
      NSArray *visible = ma_rect(extra[numbers[@"XC_kAXXCAttributeVisibleFrame"]]);
      if (visible) d[@"vf"] = visible;
    }
  }
  NSArray *children = snap.children;
  if (children.count) {
    NSMutableArray *c = [NSMutableArray arrayWithCapacity:children.count];
    for (id<XCUIElementSnapshot> child in children) [c addObject:ma_node(child, numbers)];
    d[@"c"] = c;
  }
  return d;
}

// The tree of pid's application element, as JSON: -[XCAXClient_iOS
// requestSnapshotForElement:attributes:parameters:error:], which asks
// testmanagerd, which reads it with its own privileges, since the runner's
// sandbox denies it the application's accessibility server. Modal views are
// not honoured, so that what an alert covers is read too, as the Simulator
// reader's last read keeps it. Off the main thread: the call waits for
// testmanagerd's reply.
int ma_tree(int pid, char **json, char **err) {
  @autoreleasepool {
    @try {
      id<MAAXClient> client = [XCUIDevice.sharedDevice accessibilityInterface];
      if (![client respondsToSelector:@selector(requestSnapshotForElement:attributes:parameters:error:)]) {
        return ma_fail(err, [NSString stringWithFormat:@"%@ has no requestSnapshotForElement:attributes:parameters:error:", NSStringFromClass([client class])]);
      }
      NSDictionary<NSString *, id> *numbers = ma_attribute_numbers();
      if (!numbers) return ma_fail(err, @"XCAXAccessibilityAttributesForStringAttributes is missing or names no number");
      XCAccessibilityElement *element = [NSClassFromString(@"XCAccessibilityElement") elementWithProcessIdentifier:pid];
      NSMutableDictionary *parameters = [NSMutableDictionary dictionary];
      if ([client respondsToSelector:@selector(defaultParameters)]) [parameters addEntriesFromDictionary:[client defaultParameters] ?: @{}];
      parameters[@"snapshotKeyHonorModalViews"] = @NO;
      NSError *error = nil;
      id result = [client requestSnapshotForElement:element attributes:ma_tree_attributes() parameters:parameters error:&error];
      if (!result) return ma_fail(err, ma_describe(error));
      id root = [result valueForKey:@"_rootElementSnapshot"];
      if (!root) return ma_fail(err, [NSString stringWithFormat:@"the %@ holds no root snapshot", NSStringFromClass([result class])]);
      NSString *text = ma_json(ma_node(root, numbers));
      if (!text) return ma_fail(err, @"the snapshot did not serialize as JSON");
      *json = ma_strdup(text);
      return 0;
    } @catch (NSException *e) {
      return ma_fail(err, ma_exception(e));
    }
  }
}

// ---------------------------------------------------------------------------
// Taps and swipes, in the portrait screen's points.

// Sends one event record to XCUIDevice's event synthesizer, the daemon
// session, and waits for its completion, the acknowledgement. The idle waits
// are no-ops, so nothing waits for the application to go idle.
static int ma_synthesize(XCSynthesizedEventRecord *record, char **err) {
  id<MAEventSynthesizer> synthesizer = [XCUIDevice.sharedDevice eventSynthesizer];
  if (!synthesizer) return ma_fail(err, @"XCUIDevice has no event synthesizer");
  dispatch_semaphore_t sem = dispatch_semaphore_create(0);
  __block atomic_bool done = false;
  __block NSError *error = nil;
  [synthesizer synthesizeEvent:record
                    completion:^(BOOL ok, NSError *e) {
                      error = e;
                      atomic_store(&done, true);
                      dispatch_semaphore_signal(sem);
                    }];
  if (!ma_wait(sem, &done, 20)) return ma_fail(err, @"the event synthesizer did not complete within 20 s");
  if (error) return ma_fail(err, ma_describe(error));
  return 0;
}

static XCSynthesizedEventRecord *ma_record(NSString *name, XCPointerEventPath *path) {
  XCSynthesizedEventRecord *record = [[NSClassFromString(@"XCSynthesizedEventRecord") alloc]
       initWithName:name
      interfaceOrientation:UIInterfaceOrientationPortrait];
  [record addPointerEventPath:path];
  return record;
}

// A touch at (x, y), lifted after hold seconds.
int ma_tap(double x, double y, double hold, char **err) {
  @autoreleasepool {
    @try {
      XCPointerEventPath *path = [[NSClassFromString(@"XCPointerEventPath") alloc] initForTouchAtPoint:CGPointMake(x, y) offset:0];
      [path liftUpAtOffset:hold];
      return ma_synthesize(ma_record(@"tap", path), err);
    } @catch (NSException *e) {
      return ma_fail(err, ma_exception(e));
    }
  }
}

// A touch from (x1, y1) to (x2, y2) over duration seconds. With rest above 0
// it stays at the end that long, then creeps on along its line three moves of
// a point 16 ms apart and lifts at the last, as the host's swipes through
// dtuhidd do: UIKit takes a drag's velocity from its last moves, so the
// content stops where the finger left it (crates/ios, Input).
int ma_swipe(double x1, double y1, double x2, double y2, double duration, double rest, char **err) {
  @autoreleasepool {
    @try {
      XCPointerEventPath *path = [[NSClassFromString(@"XCPointerEventPath") alloc] initForTouchAtPoint:CGPointMake(x1, y1) offset:0];
      [path moveToPoint:CGPointMake(x2, y2) atOffset:duration];
      double end = duration;
      if (rest > 0) {
        double dx = x2 - x1, dy = y2 - y1;
        double length = sqrt(dx * dx + dy * dy);
        double ux = length > 0 ? dx / length : 0, uy = length > 0 ? dy / length : 0;
        end = duration + rest;
        [path moveToPoint:CGPointMake(x2, y2) atOffset:end];
        for (int step = 1; step <= 3; step++) {
          end += 0.016;
          [path moveToPoint:CGPointMake(x2 + ux * step, y2 + uy * step) atOffset:end];
        }
      }
      [path liftUpAtOffset:end];
      return ma_synthesize(ma_record(@"swipe", path), err);
    } @catch (NSException *e) {
      return ma_fail(err, ma_exception(e));
    }
  }
}

// -[XCUIDevice pressButton:] with an XCUIDeviceButton value.
int ma_button(int button, char **err) {
  @autoreleasepool {
    __block NSString *failure = nil;
    ma_on_main(^{
      @try {
        [XCUIDevice.sharedDevice pressButton:(XCUIDeviceButton)button];
      } @catch (NSException *e) {
        failure = ma_exception(e);
      }
    });
    return failure ? ma_fail(err, failure) : 0;
  }
}

// ---------------------------------------------------------------------------
// Text.

// Types text into the element with keyboard focus through the daemon proxy's
// _XCT_sendString:maximumFrequency:completion:, as WebDriverAgent's FBKeyboard
// does; the completion is the acknowledgement.
int ma_type(const char *text, unsigned long long frequency, char **err) {
  @autoreleasepool {
    NSString *s = [NSString stringWithUTF8String:text ?: ""];
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block atomic_bool done = false;
    __block NSError *error = nil;
    @try {
      [ma_daemon() _XCT_sendString:s
                  maximumFrequency:(NSUInteger)frequency
                        completion:^(NSError *e) {
                          error = e;
                          atomic_store(&done, true);
                          dispatch_semaphore_signal(sem);
                        }];
    } @catch (NSException *e) {
      return ma_fail(err, ma_exception(e));
    }
    if (!ma_wait(sem, &done, 60)) return ma_fail(err, @"typing did not complete within 60 s");
    if (error) return ma_fail(err, ma_describe(error));
    return 0;
  }
}

// The accessibility element of the node of bundle's application that has
// keyboard focus, from XCTest's snapshot; nil when none has. Main thread.
static XCAccessibilityElement *ma_focused(NSString *bundle, NSString **failure) {
  NSError *error = nil;
  id snap = [ma_app(bundle) snapshotWithError:&error];
  if (!snap) {
    *failure = ma_describe(error);
    return nil;
  }
  NSMutableArray *queue = [NSMutableArray arrayWithObject:snap];
  while (queue.count) {
    id node = queue.firstObject;
    [queue removeObjectAtIndex:0];
    if ([node respondsToSelector:@selector(hasKeyboardFocus)] &&
        ((BOOL (*)(id, SEL))objc_msgSend)(node, @selector(hasKeyboardFocus))) {
      return [node valueForKey:@"accessibilityElement"];
    }
    [queue addObjectsFromArray:((id<XCUIElementSnapshot>)node).children ?: @[]];
  }
  *failure = @"no element has keyboard focus; tap a field first";
  return nil;
}

// Sets the value of the element of bundle's application that has keyboard
// focus to text, through the daemon proxy's _XCT_setAttribute:value:element:reply:
// with the value attribute's number.
int ma_set_text(const char *bundle, const char *text, char **err) {
  @autoreleasepool {
    NSString *bid = [NSString stringWithUTF8String:bundle ?: ""];
    NSString *value = [NSString stringWithUTF8String:text ?: ""];
    __block XCAccessibilityElement *element = nil;
    __block NSString *failure = nil;
    ma_on_main(^{
      @try {
        NSString *reason = nil;
        element = ma_focused(bid, &reason);
        if (!element) failure = reason;
      } @catch (NSException *e) {
        failure = ma_exception(e);
      }
    });
    if (failure) return ma_fail(err, failure);
    dispatch_semaphore_t sem = dispatch_semaphore_create(0);
    __block atomic_bool done = false;
    __block NSError *error = nil;
    __block BOOL ok = YES;
    @try {
      [ma_daemon() _XCT_setAttribute:@(kValueAttribute)
                               value:value
                             element:element
                               reply:^(BOOL r, NSError *e) {
                                 ok = r;
                                 error = e;
                                 atomic_store(&done, true);
                                 dispatch_semaphore_signal(sem);
                               }];
    } @catch (NSException *e) {
      return ma_fail(err, ma_exception(e));
    }
    if (!ma_wait(sem, &done, 10)) return ma_fail(err, @"the daemon did not set the value within 10 s");
    if (error) return ma_fail(err, ma_describe(error));
    if (!ok) return ma_fail(err, @"the daemon refused the value without an error");
    return 0;
  }
}

// ---------------------------------------------------------------------------
// Screenshots.

// -[XCUIScreen screenshot] of the main screen as PNG. The daemon proxy's
// _XCT_requestScreenshotOfScreenWithID:withRect:uti:compressionQuality:withReply:,
// WebDriverAgent's call, is refused by XCTest 25228's daemon as a legacy
// request (spike S10).
int ma_screenshot(unsigned char **out, size_t *len, char **err) {
  @autoreleasepool {
    __block NSData *data = nil;
    __block NSString *failure = nil;
    ma_on_main(^{
      @try {
        data = [[XCUIScreen.mainScreen screenshot] PNGRepresentation];
      } @catch (NSException *e) {
        failure = ma_exception(e);
      }
    });
    if (failure) return ma_fail(err, failure);
    if (!data.length) return ma_fail(err, @"XCUIScreen gave an empty screenshot");
    unsigned char *buffer = malloc(data.length);
    memcpy(buffer, data.bytes, data.length);
    *out = buffer;
    *len = data.length;
    return 0;
  }
}

// ---------------------------------------------------------------------------
// The test.

@interface MobileAgentRunnerTests : XCTestCase
@end

@implementation MobileAgentRunnerTests

+ (void)setUp {
  // As WebDriverAgent's +setUp: queries stay in this process, and XCTest takes
  // no screenshots of its own.
  NSUserDefaults *defaults = NSUserDefaults.standardUserDefaults;
  [defaults setBool:YES forKey:@"XCTDisableRemoteQueryEvaluation"];
  [defaults setBool:YES forKey:@"DisableScreenshots"];
  [super setUp];
}

- (void)setUp {
  [super setUp];
  self.continueAfterFailure = YES;
}

// Never ends: starts the agent's listener and spins the main run loop, on
// which the agent's calls that XCTest wants on the main thread run.
- (void)testRunner {
  NSLog(@"mobile-agent runner: idle waits replaced: %@", [gReplaced componentsJoinedByString:@", "]);
  ma_screen_scale();
  int status = agent_ios_start(kAgentPort);
  NSLog(@"mobile-agent runner: the agent started with status %d on port %u", status, kAgentPort);
  while (YES) {
    @autoreleasepool {
      [[NSRunLoop mainRunLoop] runMode:NSDefaultRunLoopMode beforeDate:[NSDate distantFuture]];
    }
  }
}

@end
