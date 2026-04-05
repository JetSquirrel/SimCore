use thiserror::Error;

#[derive(Debug, Error)]
pub enum SimError {
    #[error("simulation process panicked")]
    ProcessPanicked,
}
