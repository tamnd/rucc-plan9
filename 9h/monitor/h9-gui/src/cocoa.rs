//! The `AppKit` back end of the window (plan 11.2). It calls the Objective-C runtime directly, so
//! it needs no crate. The window has one view of the class `H9View`. The view is the delegate of
//! the window, and the target of a timer that shows the screen 60 times a second. The layer of
//! the view holds the screen as a `CGImage`, and it scales the image with the nearest pixel.

use super::{COMMAND, Held, Host, KeyEvent, Scroll, buttons, key_rune, screen_point};
use std::cell::RefCell;
use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::{Arc, OnceLock};

type Id = *mut c_void;
type Sel = *const c_void;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Point {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Size {
    w: f64,
    h: f64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Rect {
    origin: Point,
    size: Size,
}

#[link(name = "objc")]
unsafe extern "C" {
    fn objc_getClass(name: *const c_char) -> Id;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_allocateClassPair(superclass: Id, name: *const c_char, extra: usize) -> Id;
    fn class_addMethod(class: Id, sel: Sel, imp: *const c_void, types: *const c_char) -> bool;
    fn objc_registerClassPair(class: Id);
    fn objc_autoreleasePoolPush() -> *mut c_void;
    fn objc_autoreleasePoolPop(pool: *mut c_void);
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {
    static kCAFilterNearest: Id;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataCreate(alloc: *const c_void, bytes: *const u8, len: isize) -> *const c_void;
    fn CFRelease(cf: *const c_void);
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGDataProviderCreateWithCFData(data: *const c_void) -> *const c_void;
    fn CGDataProviderRelease(p: *const c_void);
    fn CGColorSpaceCreateDeviceRGB() -> *const c_void;
    fn CGColorSpaceRelease(s: *const c_void);
    fn CGImageCreate(
        width: usize,
        height: usize,
        bits_per_component: usize,
        bits_per_pixel: usize,
        bytes_per_row: usize,
        space: *const c_void,
        bitmap_info: u32,
        provider: *const c_void,
        decode: *const f64,
        interpolate: bool,
        intent: i32,
    ) -> *const c_void;
    fn CGImageRelease(i: *const c_void);
}

/// `kCGBitmapByteOrder32Little | kCGImageAlphaNoneSkipFirst`: a little-endian word of
/// 0x00RRGGBB.
const XRGB: u32 = 0x2000 | 6;
/// `NSWindowStyleMask` titled, closable, miniaturizable and resizable.
const STYLE: usize = 1 | 2 | 4 | 8;
/// `NSBackingStoreBuffered`.
const BUFFERED: usize = 2;
/// `NSEventTypeApplicationDefined`.
const APP_DEFINED: usize = 15;

/// Send the message `$sel` to `$obj` with the arguments of the given types, and give the result
/// of the type `$ret`. On arm64, `objc_msgSend` takes the arguments and gives the result as a C
/// function of that type does, structures too.
macro_rules! msg {
    ($ret:ty; $obj:expr, $sel:expr $(, $a:expr => $t:ty)*) => {{
        let f = std::mem::transmute::<
            unsafe extern "C" fn(),
            unsafe extern "C" fn(Id, Sel $(, $t)*) -> $ret,
        >(objc_msgSend);
        f($obj, sel($sel) $(, $a)*)
    }};
}

/// The selector of the name `name`.
unsafe fn sel(name: &CStr) -> Sel {
    // SAFETY: the name is a C string; the runtime keeps the selector for the life of the process.
    unsafe { sel_registerName(name.as_ptr()) }
}

/// The class of the name `name`.
unsafe fn class(name: &CStr) -> Id {
    // SAFETY: the name is a C string.
    unsafe { objc_getClass(name.as_ptr()) }
}

/// An `NSString` of `s`, autoreleased.
unsafe fn string(s: &str) -> Id {
    let c = CString::new(s.replace('\0', "")).unwrap_or_default();
    // SAFETY: the argument is a C string, which NSString copies.
    unsafe { msg![Id; class(c"NSString"), c"stringWithUTF8String:", c.as_ptr() => *const c_char] }
}

/// The host of the window. `AppKit` calls the methods of the view on the main thread only, but the
/// methods are C functions with no data of their own, so the host is a global.
static HOST: OnceLock<Arc<dyn Host>> = OnceLock::new();

/// What the view keeps between the events.
#[derive(Default)]
struct State {
    generation: u64,
    pixels: Vec<u32>,
    /// The size of the screen of the last frame.
    size: (u32, u32),
    /// The last point and buttons that went to the machine.
    at: (u16, u16, u8),
    scroll: Scroll,
    /// The keys and the modifiers that are down.
    held: Held,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

fn host() -> Option<&'static Arc<dyn Host>> {
    HOST.get()
}

pub fn run(title: &str, width: u32, height: u32, h: Arc<dyn Host>) -> Result<(), String> {
    if HOST.set(h).is_err() {
        return Err("the window is open already".into());
    }
    STATE.with(|s| s.borrow_mut().size = (width, height));
    // SAFETY: each message goes to an object of a class that has the method, with the arguments
    // and the result of the method. The caller is on the main thread, as AppKit needs.
    unsafe {
        let pool = objc_autoreleasePoolPush();
        let app = msg![Id; class(c"NSApplication"), c"sharedApplication"];
        // NSApplicationActivationPolicyRegular: a window and a Dock icon, as for an app bundle.
        msg![bool; app, c"setActivationPolicy:", 0isize => isize];
        let frame = Rect {
            origin: Point::default(),
            size: Size {
                w: f64::from(width),
                h: f64::from(height),
            },
        };
        let window = msg![Id; class(c"NSWindow"), c"alloc"];
        let window = msg![Id; window, c"initWithContentRect:styleMask:backing:defer:",
            frame => Rect, STYLE => usize, BUFFERED => usize, false => bool];
        msg![(); window, c"setReleasedWhenClosed:", false => bool];
        msg![(); window, c"setTitle:", string(title) => Id];
        msg![(); window, c"setAcceptsMouseMovedEvents:", true => bool];
        msg![(); window, c"setContentAspectRatio:", frame.size => Size];
        let view = msg![Id; view_class(), c"alloc"];
        let view = msg![Id; view, c"initWithFrame:", frame => Rect];
        // A layer of our own makes the view a layer-hosting view: `AppKit` does not draw in it.
        let layer = msg![Id; class(c"CALayer"), c"layer"];
        msg![(); layer, c"setMagnificationFilter:", kCAFilterNearest => Id];
        msg![(); view, c"setLayer:", layer => Id];
        msg![(); view, c"setWantsLayer:", true => bool];
        msg![(); window, c"setContentView:", view => Id];
        msg![(); window, c"setDelegate:", view => Id];
        msg![bool; window, c"makeFirstResponder:", view => Id];
        msg![(); window, c"center"];
        msg![(); window, c"makeKeyAndOrderFront:", std::ptr::null_mut::<c_void>() => Id];
        msg![(); app, c"activateIgnoringOtherApps:", true => bool];
        msg![Id; class(c"NSTimer"),
            c"scheduledTimerWithTimeInterval:target:selector:userInfo:repeats:",
            1.0 / 60.0 => f64, view => Id, sel(c"tick:") => Sel,
            std::ptr::null_mut::<c_void>() => Id, true => bool];
        msg![(); app, c"run"];
        msg![(); window, c"orderOut:", std::ptr::null_mut::<c_void>() => Id];
        objc_autoreleasePoolPop(pool);
    }
    Ok(())
}

/// Make the class `H9View`, a subclass of `NSView`.
unsafe fn view_class() -> Id {
    type M0 = extern "C" fn(Id, Sel) -> bool;
    type M1 = extern "C" fn(Id, Sel, Id);
    type B1 = extern "C" fn(Id, Sel, Id) -> bool;
    // SAFETY: the superclass is a class, the name is new, and each method has the C type of its
    // type encoding: B is BOOL, v is void, @ is an object and : is a selector.
    unsafe {
        let c = objc_allocateClassPair(class(c"NSView"), c"H9View".as_ptr(), 0);
        let add = |name: &CStr, imp: *const c_void, types: &CStr| {
            class_addMethod(c, sel(name), imp, types.as_ptr());
        };
        add(c"acceptsFirstResponder", yes as M0 as *const c_void, c"B@:");
        add(c"acceptsFirstMouse:", yes1 as B1 as *const c_void, c"B@:@");
        add(c"isOpaque", yes as M0 as *const c_void, c"B@:");
        add(c"tick:", tick as M1 as *const c_void, c"v@:@");
        add(c"keyDown:", key_down as M1 as *const c_void, c"v@:@");
        add(c"keyUp:", key_up as M1 as *const c_void, c"v@:@");
        add(
            c"flagsChanged:",
            flags_changed as M1 as *const c_void,
            c"v@:@",
        );
        add(
            c"windowDidResignKey:",
            resign_key as M1 as *const c_void,
            c"v@:@",
        );
        add(
            c"windowWillClose:",
            will_close as M1 as *const c_void,
            c"v@:@",
        );
        add(
            c"scrollWheel:",
            scroll_wheel as M1 as *const c_void,
            c"v@:@",
        );
        for name in [
            c"mouseDown:",
            c"mouseUp:",
            c"mouseDragged:",
            c"mouseMoved:",
            c"rightMouseDown:",
            c"rightMouseUp:",
            c"rightMouseDragged:",
            c"otherMouseDown:",
            c"otherMouseUp:",
            c"otherMouseDragged:",
        ] {
            add(name, mouse as M1 as *const c_void, c"v@:@");
        }
        objc_registerClassPair(c);
        c
    }
}

extern "C" fn yes(_: Id, _: Sel) -> bool {
    true
}

extern "C" fn yes1(_: Id, _: Sel, _: Id) -> bool {
    true
}

/// The timer: show a new frame of the screen, or stop the event loop when the session ended.
extern "C" fn tick(view: Id, _: Sel, _: Id) {
    let Some(h) = host() else { return };
    if h.done() {
        // SAFETY: the messages go to the application and to the class NSEvent with their
        // arguments. `stop:` takes effect after the next event, so post one.
        unsafe {
            let app = msg![Id; class(c"NSApplication"), c"sharedApplication"];
            msg![(); app, c"stop:", std::ptr::null_mut::<c_void>() => Id];
            let ev = msg![Id; class(c"NSEvent"),
                c"otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:",
                APP_DEFINED => usize, Point::default() => Point, 0usize => usize, 0.0 => f64,
                0isize => isize, std::ptr::null_mut::<c_void>() => Id, 0i16 => i16,
                0isize => isize, 0isize => isize];
            msg![(); app, c"postEvent:atStart:", ev => Id, true => bool];
        }
        return;
    }
    STATE.with(|s| {
        let s = &mut *s.borrow_mut();
        if let Some((w, hgt)) = h.frame(&mut s.generation, &mut s.pixels)
            && s.pixels.len() == w as usize * hgt as usize
        {
            s.size = (w, hgt);
            // SAFETY: the view has a layer, and the pixels are w by h words.
            unsafe { show(view, w, hgt, &s.pixels) };
        }
    });
}

/// Give the layer of `view` an image of the pixels.
unsafe fn show(view: Id, w: u32, h: u32, pixels: &[u32]) {
    let (w, h) = (w as usize, h as usize);
    // SAFETY: CFDataCreate copies the bytes of the slice. Each object made here is released
    // after the layer took its own reference to the image.
    unsafe {
        let data = CFDataCreate(
            std::ptr::null(),
            pixels.as_ptr().cast::<u8>(),
            isize::try_from(pixels.len() * 4).unwrap_or(0),
        );
        let provider = CGDataProviderCreateWithCFData(data);
        let space = CGColorSpaceCreateDeviceRGB();
        let image = CGImageCreate(
            w,
            h,
            8,
            32,
            w * 4,
            space,
            XRGB,
            provider,
            std::ptr::null(),
            false,
            0,
        );
        let ca = class(c"CATransaction");
        msg![(); ca, c"begin"];
        msg![(); ca, c"setDisableActions:", true => bool];
        let layer = msg![Id; view, c"layer"];
        msg![(); layer, c"setContents:", image.cast_mut() => Id];
        msg![(); ca, c"commit"];
        CGImageRelease(image);
        CGColorSpaceRelease(space);
        CGDataProviderRelease(provider);
        CFRelease(data);
    }
}

/// The point of the screen for the event `ev` in `view`.
unsafe fn point(view: Id, ev: Id) -> (u16, u16) {
    // SAFETY: `ev` is a mouse event and `view` is a view.
    unsafe {
        let p = msg![Point; ev, c"locationInWindow"];
        let p = msg![Point; view, c"convertPoint:fromView:", p => Point,
            std::ptr::null_mut::<c_void>() => Id];
        let b = msg![Rect; view, c"bounds"];
        let (w, h) = STATE.with(|s| s.borrow().size);
        screen_point(p.x, p.y, b.size.w, b.size.h, w, h)
    }
}

extern "C" fn mouse(view: Id, _: Sel, ev: Id) {
    let Some(host) = host() else { return };
    // SAFETY: `ev` is a mouse event.
    let (x, y, b) = unsafe {
        let (x, y) = point(view, ev);
        let pressed = msg![usize; class(c"NSEvent"), c"pressedMouseButtons"];
        let flags = msg![usize; ev, c"modifierFlags"];
        (x, y, buttons(pressed as u64, flags as u64))
    };
    STATE.with(|s| {
        let s = &mut *s.borrow_mut();
        if b & !s.at.2 != 0 {
            s.held.mouse_down();
        }
        s.at = (x, y, b);
    });
    host.mouse(x, y, b);
}

extern "C" fn scroll_wheel(_: Id, _: Sel, ev: Id) {
    let Some(h) = host() else { return };
    // SAFETY: `ev` is a scroll event.
    let (dy, precise) = unsafe {
        (
            msg![f64; ev, c"scrollingDeltaY"],
            msg![bool; ev, c"hasPreciseScrollingDeltas"],
        )
    };
    let (clicks, (x, y, b)) = STATE.with(|s| {
        let s = &mut *s.borrow_mut();
        (s.scroll.add(dy, precise), s.at)
    });
    for c in clicks {
        h.mouse(x, y, b | c);
        h.mouse(x, y, b);
    }
}

/// Give the key events of `f` to the host. `f` gets the keys that are down.
fn with_held(h: &Arc<dyn Host>, f: impl FnOnce(&mut Held) -> Vec<KeyEvent>) {
    for (r, down) in STATE.with(|s| f(&mut s.borrow_mut().held)) {
        h.key(r, down);
    }
}

extern "C" fn key_down(_: Id, _: Sel, ev: Id) {
    let Some(h) = host() else { return };
    // SAFETY: `ev` is a key event, and its characters are an NSString.
    unsafe {
        let flags = msg![usize; ev, c"modifierFlags"] as u64;
        if flags & COMMAND != 0 {
            let c = msg![Id; ev, c"charactersIgnoringModifiers"];
            if !c.is_null()
                && msg![usize; c, c"length"] > 0
                && msg![u16; c, c"characterAtIndex:", 0usize => usize] == u16::from(b'q')
            {
                h.quit();
            }
            return;
        }
        let code = u32::from(msg![u16; ev, c"keyCode"]);
        let chars = msg![Id; ev, c"characters"];
        let n = if chars.is_null() {
            0
        } else {
            msg![usize; chars, c"length"]
        };
        if n == 0 {
            // A dead key of the input method: the next key gives the character.
            with_held(h, |held| held.press(code, 0));
            return;
        }
        let first = msg![u16; chars, c"characterAtIndex:", 0usize => usize];
        let rune = match key_rune(first) {
            Some(r) => Some(r),
            None if n == 1 => char::from_u32(u32::from(first)).map(u32::from),
            None => None,
        };
        // Text of the input method, or a rune outside the first plane, has no key of its own: it
        // gives a key down and a key up for each rune.
        with_held(h, |held| held.press(code, rune.unwrap_or(0)));
        if rune.is_none() {
            let p = msg![*const c_char; chars, c"UTF8String"];
            if !p.is_null() {
                h.keys(CStr::from_ptr(p).to_bytes());
            }
        }
    }
}

extern "C" fn key_up(_: Id, _: Sel, ev: Id) {
    let Some(h) = host() else { return };
    // SAFETY: `ev` is a key event.
    let code = u32::from(unsafe { msg![u16; ev, c"keyCode"] });
    with_held(h, |held| held.release(code));
}

extern "C" fn flags_changed(_: Id, _: Sel, ev: Id) {
    let Some(h) = host() else { return };
    // SAFETY: `ev` is a key event.
    let flags = unsafe { msg![usize; ev, c"modifierFlags"] } as u64;
    with_held(h, |held| held.modifiers(flags));
}

extern "C" fn resign_key(_: Id, _: Sel, _: Id) {
    let Some(h) = host() else { return };
    with_held(h, Held::release_all);
}

extern "C" fn will_close(_: Id, _: Sel, _: Id) {
    if let Some(h) = host() {
        h.quit();
    }
}
