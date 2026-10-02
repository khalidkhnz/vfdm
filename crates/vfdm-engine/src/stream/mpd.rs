//! MPEG-DASH manifests. Implemented in a later milestone.

use super::Plan;
use crate::download::RunCtx;
use crate::error::{EngineError, Result};
use crate::types::DownloadRequest;

pub async fn load(_ctx: &RunCtx, _request: &DownloadRequest) -> Result<Plan> {
    Err(EngineError::Other(
        "DASH downloads are not implemented yet".into(),
    ))
}
