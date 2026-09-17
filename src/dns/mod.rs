use std::net::{Ipv4Addr, Ipv6Addr};

use futures::future::BoxFuture;

use crate::{BoxError, conf::DnsProviderKind};

#[cfg(feature = "route53")]
mod route53;
mod simulated;

pub trait DnsProvider {
    fn update_dns_record<'a>(
        &'a self,
        hostname: &'a str,
        ttl: u32,
        v4_addrs: Option<Vec<Ipv4Addr>>,
        v6_addrs: Option<Vec<Ipv6Addr>>,
    ) -> BoxFuture<'a, Result<(), BoxError>>;
}

type BoxDnsProvider = Box<dyn DnsProvider + Send + Sync + 'static>;

impl DnsProvider for BoxDnsProvider {
    fn update_dns_record<'a>(
        &'a self,
        hostname: &'a str,
        ttl: u32,
        v4_addrs: Option<Vec<Ipv4Addr>>,
        v6_addrs: Option<Vec<Ipv6Addr>>,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        (**self).update_dns_record(hostname, ttl, v4_addrs, v6_addrs)
    }
}

pub async fn create_provider(config: DnsProviderKind) -> Result<BoxDnsProvider, BoxError> {
    match config {
        DnsProviderKind::Simulated => Ok(Box::new(simulated::Simulated)),

        #[cfg(feature = "route53")]
        DnsProviderKind::Route53(config) => Ok(Box::new(route53::Route53::new(config).await)),
    }
}
