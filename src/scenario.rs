use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    FastSuccess,
    SlowProvider,
    DelayedFailure,
    ClientTimeout,
    RateLimited,
    InvalidRequest,
    UpstreamTimeout,
}

impl Scenario {
    pub const ALL: [Self; 7] = [
        Self::FastSuccess,
        Self::SlowProvider,
        Self::DelayedFailure,
        Self::ClientTimeout,
        Self::RateLimited,
        Self::InvalidRequest,
        Self::UpstreamTimeout,
    ];

    pub fn defaults(self) -> (u64, u16) {
        match self {
            Self::FastSuccess => (50, 200),
            Self::SlowProvider => (2_000, 200),
            Self::DelayedFailure => (800, 503),
            Self::ClientTimeout => (3_000, 200),
            Self::RateLimited => (100, 429),
            Self::InvalidRequest => (50, 400),
            Self::UpstreamTimeout => (2_000, 504),
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::FastSuccess => "A provider responds successfully with little delay.",
            Self::SlowProvider => "A slow provider eventually responds successfully.",
            Self::DelayedFailure => {
                "A provider is temporarily unavailable; exercise client retries."
            }
            Self::ClientTimeout => {
                "Returns 200 after 3 seconds; set a shorter timeout in your client."
            }
            Self::RateLimited => {
                "A provider rejects excess traffic and asks the client to retry later."
            }
            Self::InvalidRequest => {
                "A provider rejects invalid input; retrying unchanged will not help."
            }
            Self::UpstreamTimeout => "A gateway reports that its upstream provider timed out.",
        }
    }
}
