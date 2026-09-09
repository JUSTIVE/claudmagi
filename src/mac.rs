//! Thin AppKit shims for the frameless window: hide the traffic lights,
//! keep the window resizable, and move it while the user drags the board.

#![allow(non_camel_case_types)]

use objc::runtime::{Object, YES};
use objc::{class, msg_send, sel, sel_impl};

type id = *mut Object;

#[repr(C)]
#[derive(Clone, Copy)]
struct NSPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NSRect {
    origin: NSPoint,
    size: NSSize,
}

const NS_RESIZABLE_WINDOW_MASK: u64 = 1 << 3;

fn main_window() -> Option<id> {
    unsafe {
        let app: id = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return None;
        }
        let key: id = msg_send![app, keyWindow];
        if !key.is_null() {
            return Some(key);
        }
        let windows: id = msg_send![app, windows];
        let count: usize = msg_send![windows, count];
        if count == 0 {
            return None;
        }
        let win: id = msg_send![windows, objectAtIndex: 0usize];
        if win.is_null() { None } else { Some(win) }
    }
}

/// Removes the traffic-light buttons and re-enables edge resizing, which gpui
/// drops when a window is created without a titlebar.
pub fn style_frameless() {
    let Some(win) = main_window() else { return };
    unsafe {
        for button in 0u64..3 {
            let b: id = msg_send![win, standardWindowButton: button];
            if !b.is_null() {
                let _: () = msg_send![b, setHidden: YES];
            }
        }
        let mask: u64 = msg_send![win, styleMask];
        let _: () = msg_send![win, setStyleMask: mask | NS_RESIZABLE_WINDOW_MASK];
    }
}

/// Current mouse position in screen coordinates (origin bottom-left).
pub fn mouse_location() -> (f64, f64) {
    unsafe {
        let p: NSPoint = msg_send![class!(NSEvent), mouseLocation];
        (p.x, p.y)
    }
}

/// Shifts the main window by a screen-space delta.
pub fn move_window_by(dx: f64, dy: f64) {
    let Some(win) = main_window() else { return };
    unsafe {
        let frame: NSRect = msg_send![win, frame];
        let origin = NSPoint { x: frame.origin.x + dx, y: frame.origin.y + dy };
        let _: () = msg_send![win, setFrameOrigin: origin];
    }
}
