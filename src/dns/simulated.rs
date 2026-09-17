use std::{
    future::ready,
    net::{Ipv4Addr, Ipv6Addr},
};

use futures::{FutureExt, future::BoxFuture};

use crate::{BoxError, dns::DnsProvider, log};

pub struct Simulated;

impl DnsProvider for Simulated {
    fn update_dns_record<'a>(
        &'a self,
        hostname: &'a str,
        ttl: u32,
        v4_addrs: Option<Vec<Ipv4Addr>>,
        v6_addrs: Option<Vec<Ipv6Addr>>,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        log!(
            "Simulating DNS change for {hostname} with TTL {ttl}.  v4:{v4_addrs:?} v6:{v6_addrs:?}"
        );

        ready(Ok(())).boxed()
    }
}
