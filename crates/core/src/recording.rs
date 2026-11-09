use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;

use crate::geometry::Rect;
use crate::ids::WindowId;

pub const STOP_TIMEOUT: Duration = Duration::from_secs(5);
pub const STOP_TIMEOUT_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingState {
    Idle,
    Arming,
    Recording,
    Stopping,
    Encoding,
    Done,
    Failed,
}

impl RecordingState {
    pub const fn is_active(self) -> bool {
        matches!(
            self,
            Self::Arming | Self::Recording | Self::Stopping | Self::Encoding
        )
    }

    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Done | Self::Failed)
    }
}

impl fmt::Display for RecordingState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Idle => "Idle",
            Self::Arming => "Arming",
            Self::Recording => "Recording",
            Self::Stopping => "Stopping",
            Self::Encoding => "Encoding",
            Self::Done => "Done",
            Self::Failed => "Failed",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingTarget {
    Region(Rect),
    Window { window: WindowId, bounds: Rect },
}

impl RecordingTarget {
    pub const fn region(bounds: Rect) -> Self {
        Self::Region(bounds)
    }

    pub const fn window(window: WindowId, bounds: Rect) -> Self {
        Self::Window { window, bounds }
    }

    pub const fn bounds(self) -> Rect {
        match self {
            Self::Region(bounds) | Self::Window { bounds, .. } => bounds,
        }
    }

    pub const fn window_id(self) -> Option<WindowId> {
        match self {
            Self::Window { window, .. } => Some(window),
            Self::Region(_) => None,
        }
    }

    pub const fn is_window(self) -> bool {
        matches!(self, Self::Window { .. })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordingEvent {
    Start {
        target: RecordingTarget,
        output: PathBuf,
    },
    Armed,
    Started,
    Tick(u64),
    Stop,
    Stopped,
    Encoded,
    Failed(String),
    Reset,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RecordingEventKind {
    Start,
    Armed,
    Started,
    Tick,
    Stop,
    Stopped,
    Encoded,
    Failed,
    Reset,
}

impl RecordingEvent {
    pub const fn kind(&self) -> RecordingEventKind {
        match self {
            Self::Start { .. } => RecordingEventKind::Start,
            Self::Armed => RecordingEventKind::Armed,
            Self::Started => RecordingEventKind::Started,
            Self::Tick(_) => RecordingEventKind::Tick,
            Self::Stop => RecordingEventKind::Stop,
            Self::Stopped => RecordingEventKind::Stopped,
            Self::Encoded => RecordingEventKind::Encoded,
            Self::Failed(_) => RecordingEventKind::Failed,
            Self::Reset => RecordingEventKind::Reset,
        }
    }
}

impl fmt::Display for RecordingEventKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Start => "Start",
            Self::Armed => "Armed",
            Self::Started => "Started",
            Self::Tick => "Tick",
            Self::Stop => "Stop",
            Self::Stopped => "Stopped",
            Self::Encoded => "Encoded",
            Self::Failed => "Failed",
            Self::Reset => "Reset",
        };
        formatter.write_str(name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RecordingError {
    #[error("recording event {event} is not valid in state {state}")]
    InvalidTransition {
        state: RecordingState,
        event: RecordingEventKind,
    },
    #[error("the recording target is empty")]
    EmptyTarget,
    #[error("the recording output path is empty")]
    EmptyOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingMachine {
    state: RecordingState,
    elapsed_ms: u64,
    output: Option<PathBuf>,
    error: Option<String>,
    target: Option<RecordingTarget>,
}

impl Default for RecordingMachine {
    fn default() -> Self {
        Self::new()
    }
}

impl RecordingMachine {
    pub const fn new() -> Self {
        Self {
            state: RecordingState::Idle,
            elapsed_ms: 0,
            output: None,
            error: None,
            target: None,
        }
    }

    pub const fn state(&self) -> RecordingState {
        self.state
    }

    pub const fn elapsed_ms(&self) -> u64 {
        self.elapsed_ms
    }

    pub fn output(&self) -> Option<&Path> {
        self.output.as_deref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub const fn target(&self) -> Option<RecordingTarget> {
        self.target
    }

    pub const fn is_active(&self) -> bool {
        self.state.is_active()
    }

    pub const fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    pub fn reset(&mut self) {
        self.state = RecordingState::Idle;
        self.elapsed_ms = 0;
        self.output = None;
        self.error = None;
        self.target = None;
    }

    pub fn apply(&mut self, event: RecordingEvent) -> Result<(), RecordingError> {
        let state = self.state;
        let kind = event.kind();
        let invalid = || RecordingError::InvalidTransition { state, event: kind };

        if matches!(event, RecordingEvent::Reset) {
            self.reset();
            return Ok(());
        }

        if let RecordingEvent::Failed(message) = &event {
            if state.is_terminal() {
                return Err(invalid());
            }
            self.state = RecordingState::Failed;
            self.error = Some(message.clone());
            return Ok(());
        }

        match state {
            RecordingState::Idle => match event {
                RecordingEvent::Start { target, output } => {
                    if target.bounds().is_empty() {
                        return Err(RecordingError::EmptyTarget);
                    }
                    if output.as_os_str().is_empty() {
                        return Err(RecordingError::EmptyOutput);
                    }
                    self.state = RecordingState::Arming;
                    self.elapsed_ms = 0;
                    self.output = Some(output);
                    self.error = None;
                    self.target = Some(target);
                    Ok(())
                }
                _ => Err(invalid()),
            },
            RecordingState::Arming => match event {
                RecordingEvent::Armed | RecordingEvent::Started => {
                    self.state = RecordingState::Recording;
                    Ok(())
                }
                _ => Err(invalid()),
            },
            RecordingState::Recording => match event {
                RecordingEvent::Started => Ok(()),
                RecordingEvent::Tick(elapsed_ms) => {
                    self.elapsed_ms = elapsed_ms;
                    Ok(())
                }
                RecordingEvent::Stop => {
                    self.state = RecordingState::Stopping;
                    Ok(())
                }
                _ => Err(invalid()),
            },
            RecordingState::Stopping => match event {
                RecordingEvent::Stopped => {
                    self.state = RecordingState::Encoding;
                    Ok(())
                }
                _ => Err(invalid()),
            },
            RecordingState::Encoding => match event {
                RecordingEvent::Encoded => {
                    self.state = RecordingState::Done;
                    Ok(())
                }
                _ => Err(invalid()),
            },
            RecordingState::Done | RecordingState::Failed => Err(invalid()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Px;

    fn region() -> Rect {
        Rect::new(Px(100), Px(200), Px(640), Px(480))
    }

    fn target() -> RecordingTarget {
        RecordingTarget::region(region())
    }

    fn output() -> PathBuf {
        PathBuf::from(r"C:\Users\Aaron\Videos\recording.mp4")
    }

    fn start_event() -> RecordingEvent {
        RecordingEvent::Start {
            target: target(),
            output: output(),
        }
    }

    fn started_machine() -> RecordingMachine {
        let mut machine = RecordingMachine::new();
        assert_eq!(machine.apply(start_event()), Ok(()));
        assert_eq!(machine.apply(RecordingEvent::Armed), Ok(()));
        machine
    }

    fn in_state(state: RecordingState) -> RecordingMachine {
        let mut machine = RecordingMachine::new();
        match state {
            RecordingState::Idle => {}
            RecordingState::Arming => {
                assert_eq!(machine.apply(start_event()), Ok(()));
            }
            RecordingState::Recording => {
                machine = started_machine();
            }
            RecordingState::Stopping => {
                machine = started_machine();
                assert_eq!(machine.apply(RecordingEvent::Tick(1_500)), Ok(()));
                assert_eq!(machine.apply(RecordingEvent::Stop), Ok(()));
            }
            RecordingState::Encoding => {
                machine = in_state(RecordingState::Stopping);
                assert_eq!(machine.apply(RecordingEvent::Stopped), Ok(()));
            }
            RecordingState::Done => {
                machine = in_state(RecordingState::Encoding);
                assert_eq!(machine.apply(RecordingEvent::Encoded), Ok(()));
            }
            RecordingState::Failed => {
                assert_eq!(
                    machine.apply(RecordingEvent::Failed("encoder exited".to_owned())),
                    Ok(())
                );
            }
        }
        assert_eq!(machine.state(), state);
        machine
    }

    #[test]
    fn a_new_machine_is_idle_with_no_details() {
        let machine = RecordingMachine::new();
        assert_eq!(machine.state(), RecordingState::Idle);
        assert_eq!(machine.elapsed_ms(), 0);
        assert_eq!(machine.output(), None);
        assert_eq!(machine.error(), None);
        assert_eq!(machine.target(), None);
        assert!(!machine.is_active());
        assert!(!machine.is_terminal());
    }

    #[test]
    fn start_moves_idle_to_arming_and_remembers_the_target_and_output() {
        let mut machine = RecordingMachine::new();
        assert_eq!(machine.apply(start_event()), Ok(()));
        assert_eq!(machine.state(), RecordingState::Arming);
        assert_eq!(machine.target(), Some(target()));
        assert_eq!(machine.output(), Some(output().as_path()));
        assert_eq!(machine.elapsed_ms(), 0);
        assert!(machine.is_active());
    }

    #[test]
    fn start_rejects_an_empty_target_and_an_empty_output() {
        let mut machine = RecordingMachine::new();
        let empty_target = RecordingEvent::Start {
            target: RecordingTarget::region(Rect::ZERO),
            output: output(),
        };
        assert_eq!(
            machine.apply(empty_target),
            Err(RecordingError::EmptyTarget)
        );
        assert_eq!(machine.state(), RecordingState::Idle);
        let empty_output = RecordingEvent::Start {
            target: target(),
            output: PathBuf::new(),
        };
        assert_eq!(
            machine.apply(empty_output),
            Err(RecordingError::EmptyOutput)
        );
        assert_eq!(machine.state(), RecordingState::Idle);
    }

    #[test]
    fn armed_moves_arming_to_recording() {
        let mut machine = in_state(RecordingState::Arming);
        assert_eq!(machine.apply(RecordingEvent::Armed), Ok(()));
        assert_eq!(machine.state(), RecordingState::Recording);
        assert!(machine.is_active());
    }

    #[test]
    fn started_moves_arming_to_recording() {
        let mut machine = in_state(RecordingState::Arming);
        assert_eq!(machine.apply(RecordingEvent::Started), Ok(()));
        assert_eq!(machine.state(), RecordingState::Recording);
    }

    #[test]
    fn started_is_accepted_while_recording_without_leaving_it() {
        let mut machine = started_machine();
        assert_eq!(machine.apply(RecordingEvent::Started), Ok(()));
        assert_eq!(machine.state(), RecordingState::Recording);
    }

    #[test]
    fn ticks_replace_the_elapsed_time_while_recording() {
        let mut machine = started_machine();
        assert_eq!(machine.apply(RecordingEvent::Tick(1_000)), Ok(()));
        assert_eq!(machine.elapsed_ms(), 1_000);
        assert_eq!(machine.apply(RecordingEvent::Tick(2_750)), Ok(()));
        assert_eq!(machine.elapsed_ms(), 2_750);
    }

    #[test]
    fn a_tick_before_recording_is_rejected_and_does_not_accumulate() {
        let mut machine = in_state(RecordingState::Arming);
        assert!(matches!(
            machine.apply(RecordingEvent::Tick(900)),
            Err(RecordingError::InvalidTransition {
                state: RecordingState::Arming,
                event: RecordingEventKind::Tick
            })
        ));
        assert_eq!(machine.elapsed_ms(), 0);
    }

    #[test]
    fn stop_moves_recording_to_stopping_and_keeps_the_elapsed_time() {
        let mut machine = started_machine();
        assert_eq!(machine.apply(RecordingEvent::Tick(4_250)), Ok(()));
        assert_eq!(machine.apply(RecordingEvent::Stop), Ok(()));
        assert_eq!(machine.state(), RecordingState::Stopping);
        assert_eq!(machine.elapsed_ms(), 4_250);
        assert!(machine.is_active());
    }

    #[test]
    fn stopped_moves_stopping_to_encoding() {
        let mut machine = in_state(RecordingState::Stopping);
        assert_eq!(machine.apply(RecordingEvent::Stopped), Ok(()));
        assert_eq!(machine.state(), RecordingState::Encoding);
    }

    #[test]
    fn encoded_moves_encoding_to_done() {
        let mut machine = in_state(RecordingState::Encoding);
        assert_eq!(machine.apply(RecordingEvent::Encoded), Ok(()));
        assert_eq!(machine.state(), RecordingState::Done);
        assert!(machine.is_terminal());
        assert!(!machine.is_active());
        assert_eq!(machine.output(), Some(output().as_path()));
    }

    #[test]
    fn a_failure_moves_every_non_terminal_state_to_failed() {
        for state in [
            RecordingState::Idle,
            RecordingState::Arming,
            RecordingState::Recording,
            RecordingState::Stopping,
            RecordingState::Encoding,
        ] {
            let mut machine = in_state(state);
            assert_eq!(
                machine.apply(RecordingEvent::Failed("ffmpeg exited".to_owned())),
                Ok(())
            );
            assert_eq!(machine.state(), RecordingState::Failed);
            assert_eq!(machine.error(), Some("ffmpeg exited"));
            assert!(machine.is_terminal());
        }
    }

    #[test]
    fn terminal_states_reject_everything_except_reset() {
        for state in [RecordingState::Done, RecordingState::Failed] {
            for event in [
                start_event(),
                RecordingEvent::Armed,
                RecordingEvent::Started,
                RecordingEvent::Tick(10),
                RecordingEvent::Stop,
                RecordingEvent::Stopped,
                RecordingEvent::Encoded,
                RecordingEvent::Failed("again".to_owned()),
            ] {
                let mut machine = in_state(state);
                let before = machine.clone();
                assert!(matches!(
                    machine.apply(event),
                    Err(RecordingError::InvalidTransition { .. })
                ));
                assert_eq!(machine, before);
            }
        }
    }

    #[test]
    fn reset_returns_every_state_to_idle_and_clears_every_field() {
        for state in [
            RecordingState::Idle,
            RecordingState::Arming,
            RecordingState::Recording,
            RecordingState::Stopping,
            RecordingState::Encoding,
            RecordingState::Done,
            RecordingState::Failed,
        ] {
            let mut machine = in_state(state);
            assert_eq!(machine.apply(RecordingEvent::Reset), Ok(()));
            assert_eq!(machine.state(), RecordingState::Idle);
            assert_eq!(machine.elapsed_ms(), 0);
            assert_eq!(machine.output(), None);
            assert_eq!(machine.error(), None);
            assert_eq!(machine.target(), None);
        }
    }

    #[test]
    fn invalid_transitions_leave_the_state_and_details_untouched() {
        let cases = [
            (RecordingState::Idle, RecordingEvent::Armed),
            (RecordingState::Idle, RecordingEvent::Started),
            (RecordingState::Idle, RecordingEvent::Tick(1)),
            (RecordingState::Idle, RecordingEvent::Stop),
            (RecordingState::Idle, RecordingEvent::Stopped),
            (RecordingState::Idle, RecordingEvent::Encoded),
            (RecordingState::Arming, RecordingEvent::Tick(1)),
            (RecordingState::Arming, RecordingEvent::Stop),
            (RecordingState::Arming, RecordingEvent::Stopped),
            (RecordingState::Arming, RecordingEvent::Encoded),
            (RecordingState::Recording, RecordingEvent::Armed),
            (RecordingState::Recording, RecordingEvent::Stopped),
            (RecordingState::Recording, RecordingEvent::Encoded),
            (RecordingState::Stopping, RecordingEvent::Armed),
            (RecordingState::Stopping, RecordingEvent::Started),
            (RecordingState::Stopping, RecordingEvent::Tick(1)),
            (RecordingState::Stopping, RecordingEvent::Stop),
            (RecordingState::Stopping, RecordingEvent::Encoded),
            (RecordingState::Encoding, RecordingEvent::Armed),
            (RecordingState::Encoding, RecordingEvent::Started),
            (RecordingState::Encoding, RecordingEvent::Tick(1)),
            (RecordingState::Encoding, RecordingEvent::Stop),
            (RecordingState::Encoding, RecordingEvent::Stopped),
        ];

        for (state, event) in cases {
            let mut machine = in_state(state);
            let before = machine.clone();
            let kind = event.kind();
            assert_eq!(
                machine.apply(event),
                Err(RecordingError::InvalidTransition { state, event: kind })
            );
            assert_eq!(machine, before);
        }
    }

    #[test]
    fn a_window_target_keeps_its_handle_and_bounds() {
        let window = RecordingTarget::window(WindowId::new(42), region());
        assert_eq!(window.window_id(), Some(WindowId::new(42)));
        assert_eq!(window.bounds(), region());
        assert!(window.is_window());
        assert_eq!(target().window_id(), None);
        assert!(!target().is_window());
    }

    #[test]
    fn the_stopping_timeout_is_five_seconds() {
        assert_eq!(STOP_TIMEOUT_MS, 5_000);
        assert_eq!(STOP_TIMEOUT, Duration::from_millis(STOP_TIMEOUT_MS));
    }

    #[test]
    fn done_keeps_the_output_and_elapsed_time() {
        let machine = in_state(RecordingState::Done);
        assert_eq!(machine.output(), Some(output().as_path()));
        assert_eq!(machine.elapsed_ms(), 1_500);
        assert_eq!(machine.error(), None);
        assert_eq!(machine.target(), Some(target()));
    }

    #[test]
    fn a_failed_machine_keeps_the_target_and_output_for_recovery() {
        let mut machine = started_machine();
        assert_eq!(machine.apply(RecordingEvent::Tick(700)), Ok(()));
        assert_eq!(
            machine.apply(RecordingEvent::Failed("stopping timed out".to_owned())),
            Ok(())
        );
        assert_eq!(machine.error(), Some("stopping timed out"));
        assert_eq!(machine.output(), Some(output().as_path()));
        assert_eq!(machine.target(), Some(target()));
        assert_eq!(machine.elapsed_ms(), 700);
    }

    #[test]
    fn transition_errors_name_the_state_and_event() {
        let mut machine = RecordingMachine::new();
        let error = machine.apply(RecordingEvent::Stop);
        assert_eq!(
            error.err().map(|error| error.to_string()),
            Some("recording event Stop is not valid in state Idle".to_owned())
        );
    }
}
