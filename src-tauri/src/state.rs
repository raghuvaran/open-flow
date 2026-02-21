use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Phase {
    Idle,
    Listening,
    Processing,
    Injecting,
}

#[derive(Debug)]
pub struct AppState {
    pub phase: Phase,
}

impl AppState {
    pub fn new() -> Self {
        Self { phase: Phase::Idle }
    }

    pub fn set_listening(&mut self) {
        self.phase = Phase::Listening;
    }

    pub fn set_processing(&mut self) {
        self.phase = Phase::Processing;
    }

    pub fn set_injecting(&mut self) {
        self.phase = Phase::Injecting;
    }

    pub fn set_idle(&mut self) {
        self.phase = Phase::Idle;
    }
}
