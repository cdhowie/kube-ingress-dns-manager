use std::{
    collections::HashMap,
    hash::Hash,
    net::{Ipv4Addr, Ipv6Addr},
};

use futures::{FutureExt, future::BoxFuture};
use serde::{Deserialize, Serialize};

use crate::{BoxError, HTTP_CLIENT, conf::CloudflareConfig, dns::DnsProvider};

pub struct CloudflareProvider {
    zone_id: String,
    api_token: String,
}

impl CloudflareProvider {
    pub fn new(config: CloudflareConfig) -> Self {
        Self {
            zone_id: config.zone_id,
            api_token: config.api_token,
        }
    }
}

#[derive(Deserialize)]
struct CloudflareResponse<T> {
    result: T,
}

#[derive(Deserialize, Serialize)]
struct CloudflareDnsRecordWithId {
    id: String,
    #[serde(flatten)]
    record: CloudflareDnsRecord,
}

#[derive(Deserialize, Serialize)]
struct CloudflareDnsRecord {
    name: String,
    #[serde(flatten)]
    content: RecordContent,
    ttl: u32,
    proxied: bool,
}

#[derive(Deserialize, Serialize)]
#[allow(clippy::upper_case_acronyms)]
#[serde(tag = "type")]
enum RecordContent {
    A { content: Ipv4Addr },
    AAAA { content: Ipv6Addr },

    // We don't care about the other types, they are just here so those records
    // will parse.
    CAA,
    CERT,
    CNAME,
    DNSKEY,
    DS,
    HTTPS,
    LOC,
    MX,
    NAPTR,
    NS,
    OPENPGPKEY,
    PTR,
    SMIMEA,
    SRV,
    SSHFP,
    SVCB,
    TLSA,
    TXT,
    URI,
}

#[derive(Serialize)]
struct CloudflareBatchRecordUpdate {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    deletes: Vec<CloudflareRecordDelete>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    posts: Vec<CloudflareDnsRecord>,
}

impl From<Ipv4Addr> for RecordContent {
    fn from(value: Ipv4Addr) -> Self {
        Self::A { content: value }
    }
}

impl From<Ipv6Addr> for RecordContent {
    fn from(value: Ipv6Addr) -> Self {
        Self::AAAA { content: value }
    }
}

#[derive(Serialize)]
struct CloudflareRecordDelete {
    id: String,
}

async fn get_zone_records(
    api_token: &str,
    zone_id: &str,
    hostname: &str,
) -> Result<Vec<CloudflareDnsRecordWithId>, BoxError> {
    let response: CloudflareResponse<_> = HTTP_CLIENT
        .get(format!(
            "https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records?name.exact={hostname}"
        ))
        .header("authorization", format!("Bearer {api_token}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    Ok(response.result)
}

async fn batch_update_records(
    api_token: &str,
    zone_id: &str,
    batch: &CloudflareBatchRecordUpdate,
) -> Result<(), BoxError> {
    HTTP_CLIENT
        .post(format!(
            "https://api.cloudflare.com/client/v4/zones/{zone_id}/dns_records/batch",
        ))
        .header("authorization", format!("Bearer {api_token}"))
        .json(batch)
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

fn compute_changes<T: Eq + Hash + Into<RecordContent>>(
    hostname: &str,
    ttl: u32,
    new_addrs: Option<Vec<T>>,
    mut existing_addrs: HashMap<T, String>,
    deletes: &mut Vec<CloudflareRecordDelete>,
    posts: &mut Vec<CloudflareDnsRecord>,
) {
    let mut new_addrs = match new_addrs {
        Some(a) if !a.is_empty() => a,
        _ => return,
    };

    // This removes from new_addrs and existing_addrs the addresses present in
    // both.  After this, existing_addrs will contain addresses we want to
    // remove, and new_addrs will contain addresses we want to add.
    new_addrs.retain(|a| existing_addrs.remove(a).is_none());

    deletes.extend(
        existing_addrs
            .into_values()
            .map(|id| CloudflareRecordDelete { id }),
    );

    posts.extend(new_addrs.into_iter().map(|addr| CloudflareDnsRecord {
        name: hostname.to_owned(),
        content: addr.into(),
        ttl,
        proxied: true,
    }));
}

impl DnsProvider for CloudflareProvider {
    fn update_dns_record<'a>(
        &'a self,
        hostname: &'a str,
        ttl: u32,
        v4_addrs: Option<Vec<Ipv4Addr>>,
        v6_addrs: Option<Vec<Ipv6Addr>>,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        async move {
            // First get existing records and sort them out into v4/v6 types.
            // These map from the IP address to the Cloudflare record ID.
            let mut existing_v4 = HashMap::new();
            let mut existing_v6 = HashMap::new();

            for record in get_zone_records(&self.api_token, &self.zone_id, hostname).await? {
                match record.record.content {
                    RecordContent::A { content } => {
                        existing_v4.insert(content, record.id);
                    }
                    RecordContent::AAAA { content } => {
                        existing_v6.insert(content, record.id);
                    }
                    _ => {}
                };
            }

            let mut deletes = vec![];
            let mut posts = vec![];

            compute_changes(
                hostname,
                ttl,
                v4_addrs,
                existing_v4,
                &mut deletes,
                &mut posts,
            );

            compute_changes(
                hostname,
                ttl,
                v6_addrs,
                existing_v6,
                &mut deletes,
                &mut posts,
            );

            if !deletes.is_empty() || !posts.is_empty() {
                batch_update_records(
                    &self.api_token,
                    &self.zone_id,
                    &CloudflareBatchRecordUpdate { deletes, posts },
                )
                .await?;
            }

            Ok(())
        }
        .boxed()
    }
}
