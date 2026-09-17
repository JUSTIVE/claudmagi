//! The macOS shims: thin AppKit calls for the frameless window — hide the
//! traffic lights, keep the window resizable, move it while the user drags the
//! board — and the one thing a bundle has to fix about how it was launched.

#![allow(non_camel_case_types)]

use objc::runtime::{Object, YES};
use objc::{class, msg_send, sel, sel_impl};

type id = *mut Object;

/// Where a command-line tool lands when a shell's profile put it on `PATH`
/// and launchd's environment did not. Homebrew first on Apple silicon, then
/// the Intel/`/usr/local` prefix, then the two user-level `bin` directories.
const SHELL_PREFIXES: [&str; 4] =
    ["/opt/homebrew/bin", "/opt/homebrew/sbin", "/usr/local/bin", "/usr/local/sbin"];
const HOME_PREFIXES: [&str; 2] = [".local/bin", ".cargo/bin"];

/// Puts back the `PATH` entries a login shell would have added (#66).
///
/// An app launched from Finder, Spotlight or `open` inherits launchd's
/// environment, not a shell's, so `PATH` comes up as
/// `/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin` however the terminal that
/// built it was set up. `git`, `ps`, `open` and `orca` all live inside that
/// and kept working; `gh` lives in Homebrew's `/opt/homebrew/bin` and did
/// not, so in the installed app every pull request resolved to "no `gh`, no
/// connector" while `--prs` from a terminal listed them all.
///
/// # Safety
///
/// Call it as the first thing `main` does. `set_var` is a data race against
/// any other thread reading the environment, and at that point there are none
/// — the `gh` and `orca` workers are spawned much later.
pub unsafe fn widen_path() {
    let widened = widen(&std::env::var("PATH").unwrap_or_default(), &home_prefixes());
    unsafe { std::env::set_var("PATH", widened) }
}

fn home_prefixes() -> Vec<String> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    HOME_PREFIXES.iter().map(|p| home.join(p).to_string_lossy().into_owned()).collect()
}

/// Appends the prefixes that are missing, keeping whatever order the
/// environment already had — a shell that set `PATH` deliberately keeps its
/// own precedence, and a launchd one simply gains the entries it lacks.
fn widen(current: &str, home: &[String]) -> String {
    let mut dirs: Vec<&str> = current.split(':').filter(|s| !s.is_empty()).collect();
    let extras = SHELL_PREFIXES.iter().copied().chain(home.iter().map(String::as_str));
    for extra in extras {
        if !dirs.contains(&extra) {
            dirs.push(extra);
        }
    }
    dirs.join(":")
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_path_gains_the_shell_prefixes() {
        let home = vec!["/Users/x/.local/bin".to_string()];
        let got = widen("/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin", &home);
        assert_eq!(
            got,
            "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin:/opt/homebrew/bin:/opt/homebrew/sbin:/usr/local/sbin:/Users/x/.local/bin",
            "the missing ones are appended and /usr/local/bin is not repeated"
        );
    }

    #[test]
    fn a_shell_path_keeps_its_own_precedence() {
        let got = widen("/my/tools:/opt/homebrew/bin:/usr/bin", &[]);
        assert!(got.starts_with("/my/tools:/opt/homebrew/bin:/usr/bin"), "{got}");
        assert_eq!(got.matches("/opt/homebrew/bin").count(), 1, "no duplicates");
    }

    #[test]
    fn an_empty_path_is_still_usable() {
        assert_eq!(widen("", &[]), SHELL_PREFIXES.join(":"));
    }
}
