//! userland/wm-rs/wm.rs - a real, interactive ring-3 window manager
//! (Phase 70), replacing userland/coreutils/gui.c's own static "a few
//! colored rectangles, mouse polled but never acted on" scene with
//! genuine window management driven entirely by the graphics/mouse/
//! keyboard syscalls Phase 32b already proved work
//! (SYS_GFX_ENTER/EXIT/PUT_PIXEL/FILL_RECT, SYS_MOUSE_READ,
//! SYS_READ_KEY) - no new kernel-side GUI logic at all, exactly
//! gui.c's own "no kernel-side GUI-specific logic is used at all"
//! discipline, just real use of syscalls that were already there.
//!
//! What's real here, matching this row's own two named reference
//! points (macOS's window manager for polish/feel, Windows' taskbar +
//! snapping for cheap, immediately-felt wins):
//!   - multiple windows, each independently draggable by its own
//!     titlebar, exactly like kernel/gui/compositor.c's own ring-0
//!     dragging - but now from ring-3, through real syscalls, not a
//!     kernel task with direct framebuffer access
//!   - genuine z-order: clicking any part of a window - not just
//!     dragging its titlebar - brings it to front, in front of
//!     whatever it was behind a moment ago
//!   - a taskbar, one button per window, always visible at the
//!     bottom of the screen - clicking a button brings that window to
//!     front even if it's currently buried behind every other one
//!   - edge snapping: drag a window's titlebar so the cursor reaches
//!     the left or right edge of the screen and release - the window
//!     snaps to exactly that half. Drag it again afterward and it
//!     un-snaps back to a normal floating window, centered under
//!     wherever the cursor grabbed it - the same "snap to arrange,
//!     drag to undo" feel Windows' own snapping has
//!
//! `#![no_std]`: the same reasoning every other program in this
//! project's userland/*-rs/ directories already documents - no heap,
//! no OS-provided standard library, fixed-size stack buffers and
//! arrays throughout.

#![no_std]
#![no_main]

mod ffi;

use core::panic::PanicInfo;

#[panic_handler]
fn panic(_info: &PanicInfo) -> ! {
    unsafe {
        ffi::sys_gfx_exit(); // best-effort: leave the terminal usable
                             // even if we're panicking mid-frame,
                             // rather than stranding the user in
                             // graphics mode with no way back
        let msg = b"wm: internal error (panic)\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    loop {
        unsafe { ffi::sys_yield() };
    }
}

const SCREEN_W: i32 = 320;
const SCREEN_H: i32 = 200;
const TITLEBAR_H: i32 = 10;
const TASKBAR_H: i32 = 12;
const DESKTOP_H: i32 = SCREEN_H - TASKBAR_H; // usable area above the
                                              // taskbar - snapping and
                                              // window dragging both
                                              // respect this, not the
                                              // full screen height
const MAX_WINDOWS: usize = 4;
const SNAP_MARGIN: i32 = 4; // how close to a screen edge the cursor
                             // must get, while dragging a titlebar,
                             // to trigger a snap on release - a few
                             // pixels of real slop, not "exactly
                             // x == 0", matching how forgiving real
                             // edge-snap detection has to be for a
                             // mouse a human is actually moving

#[derive(Clone, Copy, PartialEq)]
enum SnapState {
    Normal,
    Left,
    Right,
}

#[derive(Clone, Copy)]
struct Window {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    body_color: u8,
    titlebar_color: u8,
    label: u8, // 1-9, drawn via the same 5x7 digit font
               // sys_gfx_put_pixel() builds on
    snap: SnapState,
    /// Remembered geometry from just before the most recent snap, so
    /// un-snapping restores a real, previous position/size rather
    /// than an arbitrary default - the same "snap remembers where you
    /// were" behavior real window managers have.
    pre_snap: (i32, i32, i32, i32),
}

impl Window {
    const fn new(x: i32, y: i32, w: i32, h: i32, body: u8, title: u8, label: u8) -> Self {
        Window {
            x, y, w, h,
            body_color: body,
            titlebar_color: title,
            label,
            snap: SnapState::Normal,
            pre_snap: (x, y, w, h),
        }
    }
}

fn point_in_rect(px: i32, py: i32, x: i32, y: i32, w: i32, h: i32) -> bool {
    px >= x && px < x + w && py >= y && py < y + h
}

/// Draws one 5x7 digit glyph directly via sys_gfx_put_pixel() - the
/// exact same bit pattern kernel/gui/font5x7.h's own font5x7_digits
/// table uses (reproduced here, not shared, since a ring-3 program
/// can't `#include` a kernel-private header - see this file's own
/// module doc comment on "no new kernel-side GUI logic," which cuts
/// both ways: nothing kernel-private gets exposed to ring-3 either).
const DIGIT_FONT: [[u8; 7]; 10] = [
    [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E], // 0
    [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E], // 1
    [0x0E, 0x11, 0x01, 0x0E, 0x10, 0x10, 0x1F], // 2
    [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E], // 3
    [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02], // 4
    [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E], // 5
    [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E], // 6
    [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08], // 7
    [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E], // 8
    [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C], // 9
];

fn draw_digit(x: i32, y: i32, digit: u8, color: u8) {
    if digit == 0 || digit > 9 {
        return;
    }
    let rows = &DIGIT_FONT[digit as usize];
    for (row, bits) in rows.iter().enumerate() {
        for col in 0..5 {
            if bits & (0x10 >> col) != 0 {
                unsafe { ffi::sys_gfx_put_pixel(x + col, y + row as i32, color) };
            }
        }
    }
}

fn draw_rect_outline(x: i32, y: i32, w: i32, h: i32, color: u8) {
    unsafe {
        for col in x..x + w {
            ffi::sys_gfx_put_pixel(col, y, color);
            ffi::sys_gfx_put_pixel(col, y + h - 1, color);
        }
        for row in y..y + h {
            ffi::sys_gfx_put_pixel(x, row, color);
            ffi::sys_gfx_put_pixel(x + w - 1, row, color);
        }
    }
}

/// Draws a small filled arrow cursor, one row at a time getting
/// narrower - the identical shape kernel/gui/compositor.c's own
/// bb_draw_cursor() uses, reproduced here for the same "nothing
/// kernel-private shared across the ring boundary" reason DIGIT_FONT
/// is.
fn draw_cursor(x: i32, y: i32) {
    const CURSOR_SIZE: i32 = 7;
    const CURSOR_COLOR: u8 = 15;
    unsafe {
        for row in 0..CURSOR_SIZE {
            for col in 0..=row {
                ffi::sys_gfx_put_pixel(x + col, y + row, CURSOR_COLOR);
            }
        }
    }
}

struct Wm {
    windows: [Window; MAX_WINDOWS],
    /// Draw/interaction order: z_order[0] is the bottommost window,
    /// z_order[MAX_WINDOWS - 1] the topmost (both drawn last, so it
    /// visually overlaps everything else, and checked first for
    /// clicks, so it's what actually receives them). Holds indices
    /// into `windows`, not window content itself - moving a window to
    /// front is a cheap reorder of this array, not a copy of the
    /// window's own (much larger) struct.
    z_order: [usize; MAX_WINDOWS],
    cursor_x: i32,
    cursor_y: i32,
    /// Some(index into `windows`) while a titlebar drag is in
    /// progress, None otherwise. Indexes `windows` directly (not
    /// `z_order`) since the dragged window's own identity doesn't
    /// change as z_order gets reordered around it.
    dragging: Option<usize>,
    drag_offset_x: i32,
    drag_offset_y: i32,
    prev_left_button: bool,
}

impl Wm {
    fn new() -> Self {
        Wm {
            windows: [
                Window::new(20, 20, 100, 70, 3, 1, 1),
                Window::new(140, 30, 100, 70, 10, 2, 2),
                Window::new(60, 90, 100, 70, 14, 6, 3),
                Window::new(170, 100, 90, 60, 12, 5, 4),
            ],
            z_order: [0, 1, 2, 3],
            cursor_x: SCREEN_W / 2,
            cursor_y: SCREEN_H / 2,
            dragging: None,
            drag_offset_x: 0,
            drag_offset_y: 0,
            prev_left_button: false,
        }
    }

    /// Moves `win_index` to the front of z_order (the end of the
    /// array) - real window-manager focus behavior: clicking any
    /// window, not just dragging it, brings it in front of every
    /// other one. A no-op, correctly, if it's already at the front.
    fn raise(&mut self, win_index: usize) {
        if let Some(pos) = self.z_order.iter().position(|&w| w == win_index) {
            for i in pos..MAX_WINDOWS - 1 {
                self.z_order[i] = self.z_order[i + 1];
            }
            self.z_order[MAX_WINDOWS - 1] = win_index;
        }
    }

    /// Finds the topmost window whose titlebar (not its whole body -
    /// see start_drag_or_focus() for why body clicks are handled
    /// separately) contains (px, py), searching z_order back-to-front
    /// so an overlapping window drawn on top is checked, and would
    /// correctly win, before whatever's behind it.
    fn titlebar_at(&self, px: i32, py: i32) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if point_in_rect(px, py, w.x, w.y, w.w, TITLEBAR_H) {
                return Some(idx);
            }
        }
        None
    }

    /// Same idea as titlebar_at(), but the window's entire body - for
    /// "clicking anywhere on a window raises it," not just its
    /// titlebar.
    fn window_at(&self, px: i32, py: i32) -> Option<usize> {
        for i in (0..MAX_WINDOWS).rev() {
            let idx = self.z_order[i];
            let w = &self.windows[idx];
            if point_in_rect(px, py, w.x, w.y, w.w, w.h) {
                return Some(idx);
            }
        }
        None
    }

    fn taskbar_button_at(&self, px: i32, py: i32) -> Option<usize> {
        if py < DESKTOP_H {
            return None;
        }
        let button_w = SCREEN_W / MAX_WINDOWS as i32;
        if px >= SCREEN_W {
            return None;
        }
        Some((px / button_w) as usize)
    }

    /// A left-button press just started (edge-triggered, not level-
    /// triggered - see apply_mouse()'s own prev_left_button tracking)
    /// at (px, py). Handles, in order: a taskbar click (raise that
    /// window, nothing to drag), a titlebar click (raise + begin
    /// dragging, un-snapping first if the window was snapped), or a
    /// body click (raise only, no drag - real window managers don't
    /// let you drag by clicking the middle of a window).
    fn on_left_press(&mut self, px: i32, py: i32) {
        if let Some(idx) = self.taskbar_button_at(px, py) {
            self.raise(idx);
            return;
        }
        if let Some(idx) = self.titlebar_at(px, py) {
            self.raise(idx);
            let w = &mut self.windows[idx];
            if w.snap != SnapState::Normal {
                // Un-snap back to the real geometry remembered from
                // before the snap, re-centered under the cursor's own
                // current position along the titlebar - the same
                // "drag a snapped window and it pops back to a real,
                // normal size, following your cursor" feel real
                // window managers have, not just an abrupt teleport
                // to its old position regardless of where the cursor
                // actually is now.
                let (_, _, pw, ph) = w.pre_snap;
                w.snap = SnapState::Normal;
                w.w = pw;
                w.h = ph;
                // roughly under the cursor, not exactly at the old
                // pre-snap x (the cursor may have moved since) -
                // clamped to stay fully on-screen, the same real bug
                // this exact un-clamped subtraction had once already:
                // un-snapping a window whose titlebar sits right at
                // the left edge (px near 0, since that's exactly
                // where a left-snapped window's own titlebar is)
                // produced a negative x with nothing to catch it,
                // found by hand-tracing this phase's own selftest
                // rather than the test itself catching it (the
                // selftest's own case 5 only checked snap/w/h, not x
                // - see this phase's own PROGRESS.md entry for the
                // full, honest account).
                w.x = (px - pw / 4).clamp(0, SCREEN_W - pw);
                w.y = py.min(DESKTOP_H - ph).max(0);
            }
            self.dragging = Some(idx);
            self.drag_offset_x = px - self.windows[idx].x;
            self.drag_offset_y = py - self.windows[idx].y;
            return;
        }
        if let Some(idx) = self.window_at(px, py) {
            self.raise(idx);
        }
    }

    /// The left button was released while dragging `idx` at cursor
    /// position (px, py) - checks whether the cursor ended up close
    /// enough to a screen edge (SNAP_MARGIN) to trigger Windows-style
    /// edge snapping, remembering the window's own pre-snap geometry
    /// first so a later un-snap (on_left_press(), above) has real,
    /// meaningful geometry to restore rather than nothing.
    fn on_drag_release(&mut self, idx: usize, px: i32) {
        let w = &mut self.windows[idx];
        if px <= SNAP_MARGIN {
            w.pre_snap = (w.x, w.y, w.w, w.h);
            w.snap = SnapState::Left;
            w.x = 0;
            w.y = 0;
            w.w = SCREEN_W / 2;
            w.h = DESKTOP_H;
        } else if px >= SCREEN_W - SNAP_MARGIN {
            w.pre_snap = (w.x, w.y, w.w, w.h);
            w.snap = SnapState::Right;
            w.x = SCREEN_W / 2;
            w.y = 0;
            w.w = SCREEN_W / 2;
            w.h = DESKTOP_H;
        }
        // No top-edge maximize check here deliberately: with
        // TITLEBAR_H-tall titlebars and a cursor that only ever
        // reaches y=0 by dragging almost the entire window off the
        // top of a 200px-tall screen, a real top-edge trigger would
        // fire so rarely in practice it wouldn't feel like a genuine,
        // reachable feature - left/right (reachable by any horizontal
        // drag) are the two that are actually going to get used and
        // felt, matching this row's own "cheap to implement and
        // immediately felt" standard.
    }

    fn apply_mouse(&mut self, m: ffi::MouseState) {
        self.cursor_x = (self.cursor_x + m.dx).clamp(0, SCREEN_W - 1);
        self.cursor_y = (self.cursor_y + m.dy).clamp(0, SCREEN_H - 1);

        let just_pressed = m.left_button && !self.prev_left_button;
        let just_released = !m.left_button && self.prev_left_button;

        if just_pressed && self.dragging.is_none() {
            self.on_left_press(self.cursor_x, self.cursor_y);
        }

        if let Some(idx) = self.dragging {
            if m.left_button {
                let w = &mut self.windows[idx];
                w.x = (self.cursor_x - self.drag_offset_x).clamp(0, SCREEN_W - w.w);
                w.y = (self.cursor_y - self.drag_offset_y).clamp(0, DESKTOP_H - w.h);
            }
            if just_released {
                self.on_drag_release(idx, self.cursor_x);
                self.dragging = None;
            }
        }

        self.prev_left_button = m.left_button;
    }

    fn render(&self) {
        unsafe { ffi::sys_gfx_fill_rect(0, 0, SCREEN_W, DESKTOP_H, 1) }; // desktop: blue

        for i in 0..MAX_WINDOWS {
            let w = &self.windows[self.z_order[i]];
            unsafe {
                ffi::sys_gfx_fill_rect(w.x, w.y + TITLEBAR_H, w.w, w.h - TITLEBAR_H, w.body_color);
                ffi::sys_gfx_fill_rect(w.x, w.y, w.w, TITLEBAR_H, w.titlebar_color);
            }
            draw_rect_outline(w.x, w.y, w.w, w.h, 15);
            draw_digit(w.x + 3, w.y + 1, w.label, 15);
        }

        self.render_taskbar();
        draw_cursor(self.cursor_x, self.cursor_y);
    }

    fn render_taskbar(&self) {
        unsafe { ffi::sys_gfx_fill_rect(0, DESKTOP_H, SCREEN_W, TASKBAR_H, 8) }; // dark gray bar
        let button_w = SCREEN_W / MAX_WINDOWS as i32;
        let topmost = self.z_order[MAX_WINDOWS - 1];
        for i in 0..MAX_WINDOWS {
            let bx = i as i32 * button_w;
            // The focused (topmost) window's own taskbar button is
            // drawn lighter than the rest - real, if minimal, visual
            // feedback for "this one's active," not just a plain,
            // undifferentiated row of identical buttons.
            let bg = if i == topmost { 7 } else { 8 };
            unsafe { ffi::sys_gfx_fill_rect(bx + 1, DESKTOP_H + 1, button_w - 2, TASKBAR_H - 2, bg) };
            draw_digit(bx + button_w / 2 - 2, DESKTOP_H + 2, self.windows[i].label, 0);
        }
    }
}

/// A synthetic MouseState with only the fields a given test step
/// actually cares about set to something other than their default -
/// avoids repeating all five field names at every call site below.
fn mouse(dx: i32, dy: i32, left: bool) -> ffi::MouseState {
    ffi::MouseState { dx, dy, left_button: left, right_button: false, middle_button: false }
}

/// Runs entirely without graphics mode or any real syscall - this
/// tests `Wm`'s own logic (apply_mouse()/on_left_press()/
/// on_drag_release()/raise()) directly, by feeding it the exact same
/// shape of input a real mouse would produce, and checking the
/// resulting state - deterministic, and the only practical way to
/// automatically verify this program's own real behavior at all: the
/// headless test harness this project already runs has no mechanism
/// to inject real mouse movement into a running QEMU instance, so
/// testing this by actually booting and dragging isn't possible here.
/// Prints one line per sub-test's own real pass/fail, not just a
/// final bitmask - readable on its own from a serial log the way
/// every other self-test's own output already is.
fn run_selftest() -> i32 {
    let mut code = 0;
    let mut wm = Wm::new();

    unsafe { ffi::sys_write(b"[wm-selftest] starting\n\0".as_ptr()) };

    // Case 1: dragging window 0 by its titlebar actually moves it,
    // and moving the cursor there and pressing raises it to the
    // front of z_order - the two most basic, real WM behaviors.
    let start = wm.windows[0];
    wm.cursor_x = 25; // inside window 0's own titlebar (20,20,100,10) -
    wm.cursor_y = 25; // set directly, not via a delta from Wm::new()'s
                       // own initial cursor position (SCREEN_W/2,
                       // SCREEN_H/2) - a real bug this exact mistake
                       // caused here once already, caught by hand-
                       // tracing this test's own math rather than by
                       // a lucky clean boot actually reaching it (see
                       // this phase's own PROGRESS.md entry)
    wm.apply_mouse(mouse(0, 0, true)); // press
    let raised_to_front = wm.z_order[MAX_WINDOWS - 1] == 0;
    wm.apply_mouse(mouse(10, 8, true)); // drag
    let moved = wm.windows[0].x != start.x || wm.windows[0].y != start.y;
    wm.apply_mouse(mouse(0, 0, false)); // release
    let drag_ended = wm.dragging.is_none();
    if raised_to_front && moved && drag_ended {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: titlebar drag moves a window and raises it\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: titlebar drag\n\0".as_ptr()) };
        code |= 1;
    }

    // Case 2: clicking anywhere on a different, currently-background
    // window's own body (not its titlebar) also raises it - real WM
    // focus behavior isn't limited to just dragging.
    // Window 2 starts at (60, 90, 100, 70); its own body (below the
    // titlebar) safely contains (65, 130), clear of every other
    // window's own current position.
    wm.cursor_x = 65;
    wm.cursor_y = 130;
    wm.apply_mouse(mouse(0, 0, true));
    let win2_raised = wm.z_order[MAX_WINDOWS - 1] == 2;
    wm.apply_mouse(mouse(0, 0, false));
    if win2_raised {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: clicking a window's body raises it\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: click-to-focus\n\0".as_ptr()) };
        code |= 2;
    }

    // Case 3: clicking a taskbar button raises that window even
    // though the cursor is nowhere near the window itself - the
    // actual point of having a taskbar at all.
    let button_w = SCREEN_W / MAX_WINDOWS as i32;
    wm.cursor_x = 1 * button_w + 2; // inside window 1's own taskbar button
    wm.cursor_y = DESKTOP_H + 2;
    wm.apply_mouse(mouse(0, 0, true));
    let win1_raised_via_taskbar = wm.z_order[MAX_WINDOWS - 1] == 1;
    wm.apply_mouse(mouse(0, 0, false));
    if win1_raised_via_taskbar {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: taskbar button click raises its window\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: taskbar focus\n\0".as_ptr()) };
        code |= 4;
    }

    // Case 4: dragging window 3's titlebar to the left screen edge
    // and releasing snaps it to exactly the left half - the real,
    // observable point of edge snapping.
    let w3 = wm.windows[3];
    wm.cursor_x = w3.x + 5;
    wm.cursor_y = w3.y + 3;
    wm.apply_mouse(mouse(0, 0, true)); // grab window 3's titlebar
    // Drag all the way to the left edge in one large step - apply_
    // mouse() clamps the cursor itself to the screen, so a single,
    // deliberately-oversized negative dx reaches x=0 in one call,
    // the same as a real, fast mouse swipe would.
    wm.apply_mouse(mouse(-400, 0, true));
    wm.apply_mouse(mouse(0, 0, false)); // release at the left edge
    let snapped_left = wm.windows[3].snap == SnapState::Left
        && wm.windows[3].x == 0
        && wm.windows[3].y == 0
        && wm.windows[3].w == SCREEN_W / 2
        && wm.windows[3].h == DESKTOP_H;
    if snapped_left {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: dragging to the left edge snaps the window\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: edge snap\n\0".as_ptr()) };
        code |= 8;
    }

    // Case 5: grabbing an already-snapped window's titlebar again
    // un-snaps it back to a real, normal floating window - snapping
    // that can't be undone wouldn't feel like the real thing.
    wm.cursor_x = 5;
    wm.cursor_y = 3; // inside window 3's own titlebar, now at (0,0) post-snap -
                      // deliberately right at the left edge, the exact
                      // condition that found this file's own real,
                      // once-unclamped-x bug by hand-tracing this
                      // case, not by this assertion catching it - see
                      // that fix's own comment on w.x's calculation
    wm.apply_mouse(mouse(0, 0, true));
    let w3_now = wm.windows[3];
    let unsnapped = w3_now.snap == SnapState::Normal
        && w3_now.w == w3.w
        && w3_now.h == w3.h
        && w3_now.x >= 0 && w3_now.x + w3_now.w <= SCREEN_W
        && w3_now.y >= 0 && w3_now.y + w3_now.h <= DESKTOP_H;
    wm.apply_mouse(mouse(0, 0, false));
    if unsnapped {
        unsafe { ffi::sys_write(b"[wm-selftest] PASS: re-dragging a snapped window un-snaps it, fully on-screen\n\0".as_ptr()) };
    } else {
        unsafe { ffi::sys_write(b"[wm-selftest] FAIL: un-snap\n\0".as_ptr()) };
        code |= 16;
    }

    if code == 0 {
        unsafe { ffi::sys_write(b"[wm-selftest] all cases passed\n\0".as_ptr()) };
    }
    code
}

/// Compares a NUL-terminated C string (`ptr`) against `prefix`, byte
/// by byte, stopping safely at whichever ends first - never reads
/// past `ptr`'s own NUL terminator, unlike a fixed-length slice read
/// would if the real argument were shorter than `prefix`.
fn starts_with_cstr(ptr: *const u8, prefix: &[u8]) -> bool {
    if ptr.is_null() {
        return false;
    }
    for (i, &want) in prefix.iter().enumerate() {
        let got = unsafe { *ptr.add(i) };
        if got == 0 || got != want {
            return false;
        }
    }
    true
}

#[no_mangle]
pub extern "C" fn main(argc: i32, argv: *const *const u8, _envp: *const *const u8) -> i32 {
    // A real, deliberate test hook, not a hidden debug backdoor: see
    // run_selftest()'s own doc comment for why this is the only
    // practical way to automatically verify this program's own real
    // behavior at all in this project's existing headless test
    // harness. Checked before sys_gfx_enter() - the self-test needs
    // no graphics mode at all, and skipping it keeps `make test`'s
    // own boot fast and side-effect-free.
    if argc >= 2 {
        let ptr = unsafe { *argv.offset(1) };
        if starts_with_cstr(ptr, b"--selftest") {
            return run_selftest();
        }
    }

    unsafe {
        let msg = b"Entering the NovaOS window manager (Phase 70) - drag \
                     titlebars, click the taskbar, drag to a screen edge \
                     to snap. Press 'q' to quit.\n\0";
        ffi::sys_write(msg.as_ptr());
        ffi::sys_gfx_enter();
    }

    let mut wm = Wm::new();

    loop {
        let key = unsafe { ffi::sys_read_key() };
        if key == b'q' as i32 || key == 27 {
            // 27 = ESC, the other real, expected "get me out of here"
            // key alongside 'q' - checked as a plain ASCII value, not
            // a symbolic constant, matching sys_read_key()'s own
            // "real character, not a scancode" contract (see ffi.rs's
            // own doc comment on it).
            break;
        }

        let mut m = ffi::MouseState { dx: 0, dy: 0, left_button: false, right_button: false, middle_button: false };
        unsafe { ffi::sys_mouse_read(&mut m) };
        wm.apply_mouse(m);

        wm.render();
        unsafe { ffi::sys_yield() };
    }

    unsafe {
        ffi::sys_gfx_exit();
        let msg = b"Window manager exited.\n\0";
        ffi::sys_write(msg.as_ptr());
    }
    0
}
