//! yt-dlp child-process runner. Implemented in a later milestone.

use crate::download::{RunCtx, RunResult};
use std::sync::Arc;

pub async fn run(_ctx: Arc<RunCtx>) -> RunResult {
    RunResult::Failed("yt-dlp downloads are not implemented yet".into())
}
