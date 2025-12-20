// Minimal loop workflow state machine
//
// States: Starting -> Sleeping -> Running -> (loop until max iterations)
//
// Note: temporal-sdk-core 0.1.0-alpha.1 doesn't support continue-as-new or
// history pagination, so we complete after MAX_ITERATIONS to avoid history
// growth errors. In production, upgrade SDK and use continue-as-new.

use serde::{Deserialize, Serialize};

/// Max iterations before completing (to avoid history pagination issues)
const MAX_ITERATIONS: u32 = 20;

/// Workflow input (empty for now, add fields as needed)
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoopInput {}

/// Workflow state
#[derive(Debug, Clone)]
pub enum LoopState {
    Starting,
    Sleeping { timer_seq: u32 },
    RunningActivity { activity_seq: u32 },
    Completed,
}

/// Commands the workflow produces
#[derive(Debug)]
pub enum LoopCommand {
    StartTimer { seq: u32, seconds: u64 },
    ScheduleActivity { seq: u32, iteration: u32 },
    Complete { iteration: u32 },
    None,
}

/// The workflow state machine
pub struct AgentSessionWorkflow {
    iteration: u32,
    state: LoopState,
    seq: u32,
}

impl AgentSessionWorkflow {
    pub fn new(_input: LoopInput) -> Self {
        Self {
            iteration: 0,
            state: LoopState::Starting,
            seq: 0,
        }
    }

    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    fn next_seq(&mut self) -> u32 {
        self.seq += 1;
        self.seq
    }

    /// Called when workflow starts
    pub fn on_start(&mut self) -> LoopCommand {
        let seq = self.next_seq();
        self.state = LoopState::Sleeping { timer_seq: seq };
        LoopCommand::StartTimer { seq, seconds: 1800 } // 30 minutes
    }

    /// Called when timer fires
    pub fn on_timer_fired(&mut self, _seq: u32) -> LoopCommand {
        let seq = self.next_seq();
        self.state = LoopState::RunningActivity { activity_seq: seq };
        LoopCommand::ScheduleActivity {
            seq,
            iteration: self.iteration,
        }
    }

    /// Called when activity completes
    pub fn on_activity_completed(&mut self, _seq: u32) -> LoopCommand {
        self.iteration += 1;

        // Complete after max iterations (SDK limitation workaround)
        if self.iteration >= MAX_ITERATIONS {
            self.state = LoopState::Completed;
            return LoopCommand::Complete {
                iteration: self.iteration,
            };
        }

        // Start another timer
        let seq = self.next_seq();
        self.state = LoopState::Sleeping { timer_seq: seq };
        LoopCommand::StartTimer { seq, seconds: 1800 } // 30 minutes
    }

    /// Called on wake signal (skips timer, runs activity immediately)
    pub fn on_signal(&mut self) -> LoopCommand {
        match &self.state {
            LoopState::Sleeping { .. } => {
                let seq = self.next_seq();
                self.state = LoopState::RunningActivity { activity_seq: seq };
                LoopCommand::ScheduleActivity {
                    seq,
                    iteration: self.iteration,
                }
            }
            _ => LoopCommand::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_loop() {
        let mut wf = AgentSessionWorkflow::new(LoopInput {});

        // Start -> timer
        let cmd = wf.on_start();
        assert!(matches!(cmd, LoopCommand::StartTimer { seq: 1, .. }));

        // Timer fires -> activity
        let cmd = wf.on_timer_fired(1);
        assert!(matches!(cmd, LoopCommand::ScheduleActivity { seq: 2, .. }));

        // Activity completes -> timer again
        let cmd = wf.on_activity_completed(2);
        assert!(matches!(cmd, LoopCommand::StartTimer { seq: 3, .. }));
        assert_eq!(wf.iteration(), 1);
    }
}
