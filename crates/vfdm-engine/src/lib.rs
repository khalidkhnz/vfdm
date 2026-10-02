//! vfdm-engine: segmented parallel HTTP downloads with work-stealing re-split,
//! resume, and a bounded queue. No UI dependencies.

pub mod error;
pub mod filename;
pub mod http;
pub mod planner;
pub mod probe;
pub mod scheduler;
pub mod speed;
pub mod state;
pub mod stream;
pub mod types;
pub mod writer;

mod download;
mod worker;
mod ytdlp;

pub use error::EngineError;
pub use scheduler::Engine;
pub use types::*;
