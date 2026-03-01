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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_idle() {
        let s = AppState::new();
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn phase_transitions() {
        let mut s = AppState::new();
        s.set_listening();
        assert_eq!(s.phase, Phase::Listening);
        s.set_processing();
        assert_eq!(s.phase, Phase::Processing);
        s.set_injecting();
        assert_eq!(s.phase, Phase::Injecting);
        s.set_idle();
        assert_eq!(s.phase, Phase::Idle);
    }

    #[test]
    fn can_go_idle_from_any_state() {
        let mut s = AppState::new();
        s.set_listening();
        s.set_idle();
        assert_eq!(s.phase, Phase::Idle);
        s.set_processing();
        s.set_idle();
        assert_eq!(s.phase, Phase::Idle);
    }
}
