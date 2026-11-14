//! `ApiError` (docs/contracts.md#apierror): every non-2xx response body.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiError {
    pub error: String,
}

impl ApiError {
    pub fn new(msg: impl Into<String>) -> ApiError {
        ApiError { error: msg.into() }
    }
}
