use thiserror::Error;

#[derive(Error, Debug)]
pub enum FiberError {
    #[error("Config error: {0}")]
    Config(String),

    #[error("Metric error: {0}")]
    Metric(String),

    #[error("Git error: {0}")]
    Git(String),

    #[error("Report error: {0}")]
    Report(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
