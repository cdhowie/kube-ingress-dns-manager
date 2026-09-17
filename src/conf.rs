use std::num::NonZeroU64;

use serde::Deserialize;

#[derive(Deserialize)]
pub struct Config {
    pub node_labels: Option<String>,
    #[serde(default = "default_true")]
    pub skip_unschedulable: bool,
    #[serde(default = "default_5")]
    pub update_interval_sec: NonZeroU64,

    pub dns: Dns,

    pub health_check: Option<HealthCheck>,
}

#[derive(Deserialize)]
pub struct Dns {
    pub hostname: String,
    pub ttl: u32,
    pub provider: DnsProviderKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "kind")]
pub enum DnsProviderKind {
    Simulated,
    #[cfg(feature = "cloudflare")]
    Cloudflare(CloudflareConfig),
    #[cfg(feature = "route53")]
    Route53(Route53Config),
}

#[cfg(feature = "cloudflare")]
#[derive(Deserialize)]
pub struct CloudflareConfig {
    pub zone_id: String,
    pub api_token: String,
}

#[cfg(feature = "route53")]
#[derive(Deserialize)]
pub struct Route53Config {
    pub hosted_zone_id: String,
}

#[derive(Deserialize)]
pub struct HealthCheck {
    pub checks_to_up: usize,
    #[serde(default)] // 0 disables checking healthy addresses.
    pub checks_to_down: usize,

    #[serde(flatten)]
    pub kind: HealthCheckKind,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "kind")]
pub enum HealthCheckKind {
    Http(HttpHealthCheck),
}

#[derive(Deserialize)]
pub struct HttpHealthCheck {
    pub uri: String,
    #[serde(default = "default_http_port")]
    pub port: u16,
    pub timeout_sec: u64,
    pub valid_status_codes: Vec<u16>,
}

fn default_true() -> bool {
    true
}

fn default_5() -> NonZeroU64 {
    5.try_into().unwrap()
}

fn default_http_port() -> u16 {
    80
}

pub fn load() -> Result<Config, config::ConfigError> {
    config::Config::builder()
        .add_source(config::File::with_name("config"))
        .add_source(config::Environment::with_prefix("INGRESSDNS").separator("__"))
        .build()?
        .try_deserialize()
}
