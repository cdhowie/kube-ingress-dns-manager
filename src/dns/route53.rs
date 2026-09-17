use std::net::{Ipv4Addr, Ipv6Addr};

use aws_config::{BehaviorVersion, Region};
use aws_sdk_route53::{
    Client,
    types::{Change, ChangeAction, ChangeBatch, ResourceRecord, ResourceRecordSet, RrType},
};
use futures::{FutureExt, future::BoxFuture};

use crate::{BoxError, conf::Route53Config, dns::DnsProvider, log};

pub struct Route53 {
    client: Client,
    hosted_zone_id: String,
}

impl Route53 {
    pub async fn new(config: Route53Config) -> Self {
        let client = Client::new(
            &aws_config::load_defaults(BehaviorVersion::v2026_01_12())
                .await
                .into_builder()
                .region(Some(Region::from_static("us-east-1")))
                .build(),
        );

        Self {
            client,
            hosted_zone_id: config.hosted_zone_id,
        }
    }
}

trait GetRrType {
    fn get_rr_type() -> RrType;
}

impl GetRrType for Ipv4Addr {
    fn get_rr_type() -> RrType {
        RrType::A
    }
}

impl GetRrType for Ipv6Addr {
    fn get_rr_type() -> aws_sdk_route53::types::RrType {
        RrType::Aaaa
    }
}

fn create_change_for_addrs<T: GetRrType + ToString>(
    addrs: &[T],
    hostname: String,
    ttl: u32,
) -> Option<Change> {
    if addrs.is_empty() {
        return None;
    }

    Some(
        Change::builder()
            .set_action(Some(ChangeAction::Upsert))
            .set_resource_record_set(Some(
                ResourceRecordSet::builder()
                    .name(hostname)
                    .set_type(Some(T::get_rr_type()))
                    .set_ttl(Some(ttl.into()))
                    .set_resource_records(Some(
                        addrs
                            .iter()
                            .map(|a| {
                                ResourceRecord::builder()
                                    .set_value(Some(a.to_string()))
                                    .build()
                                    .unwrap()
                            })
                            .collect(),
                    ))
                    .build()
                    .unwrap(),
            ))
            .build()
            .unwrap(),
    )
}

impl DnsProvider for Route53 {
    fn update_dns_record<'a>(
        &'a self,
        hostname: &'a str,
        ttl: u32,
        v4_addrs: Option<Vec<Ipv4Addr>>,
        v6_addrs: Option<Vec<Ipv6Addr>>,
    ) -> BoxFuture<'a, Result<(), BoxError>> {
        async move {
            let v4change =
                v4_addrs.and_then(|a| create_change_for_addrs(&a, hostname.to_owned(), ttl));

            let v6change =
                v6_addrs.and_then(|a| create_change_for_addrs(&a, hostname.to_owned(), ttl));

            if v4change.is_some() || v6change.is_some() {
                let result = self
                    .client
                    .change_resource_record_sets()
                    .set_hosted_zone_id(Some(self.hosted_zone_id.clone()))
                    .set_change_batch(Some(
                        ChangeBatch::builder()
                            .set_changes(Some(v4change.into_iter().chain(v6change).collect()))
                            .build()
                            .unwrap(),
                    ))
                    .send()
                    .await?;

                if let Some(ci) = result.change_info() {
                    log!(
                        "route53: Created change {} with status {}",
                        ci.id,
                        ci.status,
                    );
                }
            }

            Ok(())
        }
        .boxed()
    }
}
