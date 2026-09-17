use std::{fmt::Display, net::IpAddr, time::Duration};

use tokio::time::timeout;

use crate::{
    HTTP_CLIENT,
    conf::{HealthCheck, HealthCheckKind, HttpHealthCheck},
};

#[derive(Debug, Clone, Copy)]
pub enum Health {
    Healthy { failures: usize },
    Unhealthy { successes: usize },
}

impl Default for Health {
    fn default() -> Self {
        Self::Unhealthy { successes: 0 }
    }
}

impl Health {
    pub fn is_up(&self) -> bool {
        matches!(self, &Self::Healthy { .. })
    }

    pub fn add_result(&self, success: bool, policy: &HealthCheck) -> Self {
        match (success, self) {
            (true, &Self::Healthy { .. }) => Self::Healthy { failures: 0 },

            (true, &Self::Unhealthy { mut successes }) => {
                successes += 1;

                if successes >= policy.checks_to_up {
                    Self::Healthy { failures: 0 }
                } else {
                    Self::Unhealthy { successes }
                }
            }

            (false, &Self::Unhealthy { .. }) => Self::Unhealthy { successes: 0 },

            (false, &Self::Healthy { mut failures }) => {
                failures += 1;

                if failures >= policy.checks_to_down {
                    Self::Unhealthy { successes: 0 }
                } else {
                    Self::Healthy { failures }
                }
            }
        }
    }
}

pub async fn check(address: IpAddr, check: &HealthCheckKind) -> bool {
    match check {
        HealthCheckKind::Http(http) => check_http(address, http).await,
    }
}

struct UrlFormattedIpAddr(IpAddr);

impl Display for UrlFormattedIpAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            IpAddr::V4(a) => write!(f, "{a}"),
            IpAddr::V6(a) => write!(f, "[{a}]"),
        }
    }
}

async fn check_http(address: IpAddr, check: &HttpHealthCheck) -> bool {
    let url = format!(
        "http://{}:{}{}",
        UrlFormattedIpAddr(address),
        check.port,
        check.uri,
    );

    timeout(Duration::from_secs(check.timeout_sec), async {
        HTTP_CLIENT
            .get(url)
            .send()
            .await
            .is_ok_and(|r| check.valid_status_codes.contains(&r.status().as_u16()))
    })
    .await
    .unwrap_or(false)
}
