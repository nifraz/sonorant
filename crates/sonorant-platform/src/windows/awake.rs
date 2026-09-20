//! Keeping the screen on while something is worth watching.
//!
//! Windows counts a keypress or a moved pointer as activity, and a full-screen analyser
//! that is playing music has neither. `SetThreadExecutionState` is how a media player
//! says so: the flags are a standing request rather than a one-off, so setting it once
//! holds the screen on until it is set back.

use windows::Win32::System::Power::{
    ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, EXECUTION_STATE,
    SetThreadExecutionState,
};

/// Holds the screen on while it is asked to.
#[derive(Debug, Default)]
pub struct ScreenAwake {
    on: bool,
}

impl ScreenAwake {
    pub fn new() -> ScreenAwake {
        ScreenAwake::default()
    }

    /// Asks for the screen to stay on, or lets it go back to the usual timeout.
    ///
    /// The request is a state rather than a pulse, so this only calls into Windows when
    /// the answer changes.
    pub fn set(&mut self, on: bool) {
        if on == self.on {
            return;
        }
        self.on = on;
        let flags = if on {
            ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED
        } else {
            ES_CONTINUOUS
        };
        // SAFETY: the call takes a bitmask by value and touches nothing of ours. A zero
        // return means it was refused, which is worth a line but not worth failing over.
        let previous = unsafe { SetThreadExecutionState(flags) };
        if previous == EXECUTION_STATE(0) {
            log::debug!(
                "Windows would not {} the screen awake",
                if on { "hold" } else { "let go of" }
            );
        } else {
            log::debug!("screen awake: {on}");
        }
    }
}

impl Drop for ScreenAwake {
    fn drop(&mut self) {
        self.set(false);
    }
}
