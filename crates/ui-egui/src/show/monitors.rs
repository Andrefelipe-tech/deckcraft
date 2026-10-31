//! Real monitor detection for the Slide Show's dual-window Presenter View.
//!
//! DeckCraft never mirrors the audience window and never guesses a screen position. With two or
//! more displays connected, `F5` opens the Presenter View on one window and a second, chrome-free
//! window carrying only the current slide; `has_second_display` asks the operating system's own
//! display list, read fresh each time the show starts, so a monitor plugged or unplugged since
//! the last show is picked up correctly. With one display (or none reachable, as on a web build or
//! a headless CI runner), the show stays a single window, exactly as it always has.

/// Whether at least two displays are connected right now. Checked once, when the show starts
/// (`Show::new`): never crashes and never blocks on a system that can't answer (an unsupported
/// platform, a headless session, or a sandboxed environment with no display server) — it just
/// says "no second display" and the show falls back to its single-window behaviour.
pub(super) fn has_second_display() -> bool {
    imp::has_second_display()
}

#[cfg(not(target_arch = "wasm32"))]
mod imp {
    pub(super) fn has_second_display() -> bool {
        // `DisplayInfo::all()` queries the OS directly (Win32 `EnumDisplayMonitors`, macOS
        // `CGGetActiveDisplayList`, X11/Wayland on Linux) — no fixed coordinates, no assumptions.
        // Any failure (no display server reachable, an unsupported backend) is "can't tell", which
        // is treated the same as "only one display": the show stays a single window rather than
        // risking a window placed nowhere useful.
        std::panic::catch_unwind(display_info::DisplayInfo::all).map(|r| r.map(|m| m.len() >= 2).unwrap_or(false)).unwrap_or(false)
    }
}

// The web build has no concept of a second OS window at all; the show is always a single tab.
#[cfg(target_arch = "wasm32")]
mod imp {
    pub(super) fn has_second_display() -> bool {
        false
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    /// Never panics, whatever the host reports (including "no display at all", as in CI).
    #[test]
    fn never_panics() {
        let _ = has_second_display();
    }
}
