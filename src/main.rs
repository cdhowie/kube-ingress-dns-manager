use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::LazyLock,
    time::Duration,
};

use futures::StreamExt;
use k8s_openapi::api::core::v1::Node;
use kube::{
    Api,
    runtime::watcher::{self, Event},
};
use tokio::time::{MissedTickBehavior, interval};

use crate::{conf::HealthCheck, dns::DnsProvider, health::Health};

mod conf;
mod dns;
mod health;

#[macro_export]
macro_rules! log {
    ( $( $arg:expr ),+ $(,)? ) => {
        println!("[{}] {}", ::chrono::Utc::now(), format_args!($($arg),+))
    };
}

static HTTP_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .hickory_dns(true)
        .build()
        .unwrap()
});

type BoxError = Box<dyn std::error::Error + Send + Sync + 'static>;

#[cfg(unix)]
async fn wait_for_terminate_signal() {
    use tokio::signal::unix::{SignalKind, signal};

    async fn wait_for(kind: SignalKind) {
        let received = match signal(kind) {
            Ok(mut v) => v.recv().await.is_some(),
            Err(_) => false,
        };

        if !received {
            // Signal could not be received for some reason.  Wait forever; the
            // alternative is we return and that incorrectly signals
            // termination.
            std::future::pending().await
        }
    }

    let term = wait_for(SignalKind::terminate());
    let int = wait_for(SignalKind::interrupt());

    tokio::select! {
        biased;
        () = term => {},
        () = int => {},
    }
}

#[cfg(not(unix))]
async fn wait_for_terminate_signal() {
    // Not supported on other platforms yet; wait forever, which will never
    // signal termination.
    std::future::pending().await
}

struct NodeState {
    addresses: Vec<IpAddr>,
}

#[derive(Default)]
struct DnsState {
    v4addrs: HashSet<Ipv4Addr>,
    v6addrs: HashSet<Ipv6Addr>,
}

fn get_node_addresses(node: &Node) -> Option<Vec<IpAddr>> {
    let addresses: Vec<IpAddr> = node
        .status
        .as_ref()?
        .addresses
        .as_ref()?
        .iter()
        .filter(|addr| addr.type_ == "ExternalIP")
        .filter_map(|addr| addr.address.parse().ok())
        .collect();

    (!addresses.is_empty()).then_some(addresses)
}

async fn do_health_checks(
    address_health: &mut HashMap<IpAddr, Health>,
    new_addresses: impl IntoIterator<Item = IpAddr>,
    check: &HealthCheck,
) {
    let mut new_addresses: HashSet<IpAddr> = new_addresses.into_iter().collect();

    // Remove addresses from the health map if they aren't present in the new
    // addresses set.  Simultaneously, remove addresses in the health map from
    // the new addresses set.
    address_health.retain(|addr, _| {
        let r = new_addresses.remove(addr);
        if !r {
            log!("Address removed: {addr}");
        }
        r
    });

    // What's left in new_addresses doesn't appear in the health map.
    address_health.extend(new_addresses.into_iter().map(|addr| {
        log!("Address added: {addr}");
        (addr, Health::default())
    }));

    // Send health checks for everything in the map.
    futures::stream::iter(address_health.iter_mut())
        .for_each_concurrent(16, async |(&addr, health)| {
            // If checks_to_down is zero, this disables health checks of healthy
            // addresses.
            if check.checks_to_down == 0 && health.is_up() {
                return;
            }

            let success = health::check(addr, &check.kind).await;

            let new_health = health.add_result(success, check);

            let un = match (health.is_up(), new_health.is_up()) {
                (false, true) => Some(""),
                (true, false) => Some("un"),
                _ => None,
            };

            if let Some(un) = un {
                log!("{addr} became {un}healthy");
            }

            *health = new_health;
        })
        .await;
}

async fn apply_state(
    dns_provider: &impl DnsProvider,
    hostname: &str,
    ttl: u32,
    existing_state: &mut DnsState,
    new_addresses: impl IntoIterator<Item = IpAddr>,
) {
    let mut new_state = DnsState::default();

    for addr in new_addresses {
        match addr {
            IpAddr::V4(a) => new_state.v4addrs.insert(a),
            IpAddr::V6(a) => new_state.v6addrs.insert(a),
        };
    }

    let v4addrs = (new_state.v4addrs != existing_state.v4addrs)
        .then(|| new_state.v4addrs.iter().copied().collect());

    let v6addrs = (new_state.v6addrs != existing_state.v6addrs)
        .then(|| new_state.v6addrs.iter().copied().collect());

    if v4addrs.is_some() || v6addrs.is_some() {
        let result = dns_provider
            .update_dns_record(hostname, ttl, v4addrs, v6addrs)
            .await;

        match result {
            Err(err) => {
                log!("Error updating DNS records: {err}");
            }

            Ok(()) => {
                log!("DNS provider updated with new records");
                *existing_state = new_state;
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let config = conf::load()?;

    let dns_provider = dns::create_provider(config.dns.provider).await?;

    let mut watcher_config = watcher::Config::default();
    if let Some(labels) = &config.node_labels {
        watcher_config = watcher_config.labels(labels);
    }

    let node_watcher = watcher::watcher(
        Api::<Node>::all(kube::Client::try_default().await?),
        watcher_config,
    );

    tokio::pin!(node_watcher);

    let terminate = wait_for_terminate_signal();

    tokio::pin!(terminate);

    let mut update_interval = interval(Duration::from_secs(config.update_interval_sec.into()));
    update_interval.set_missed_tick_behavior(MissedTickBehavior::Delay);

    // This flag will be true if applying changes is suspended.  This happens if
    // an "init" event comes through.  After initialization, this would indicate
    // the watcher stream could not be recovered and we're going to receive the
    // full data set again.  We suspend in that case until we see an InitDone
    // event so we know we are working with the full set of nodes before
    // potentially making changes to DNS.
    let mut suspend = true;

    // If health checks are enabled, we don't want to take any action until we
    // have done enough health checks that an address could be considered
    // healthy.  This keeps track of how many checks we have left to do.
    let mut warmup_checks: usize = config
        .health_check
        .as_ref()
        .map_or_default(|c| c.checks_to_up.into());

    let mut nodes: HashMap<String, NodeState> = HashMap::new();
    let mut address_health: HashMap<IpAddr, Health> = HashMap::new();
    let mut dns_state = DnsState::default();

    loop {
        tokio::select! {
            // Biased because we want to check the futures in exactly the order
            // listed here.
            //
            // * We always want to terminate immediately if signaled.
            // * We want to process as much state from the watcher as is
            //   available before we apply it to DNS.
            biased;

            () = &mut terminate => {
                break;
            }

            event = node_watcher.next() => {
                let event = match event.expect("watcher stream ended unexpectedly") {
                    Ok(e) => e,
                    Err(err) => {
                        log!("Watcher error: {err}");
                        continue;
                    }
                };

                match event {
                    Event::Init => {
                        nodes.clear();
                        suspend = true;
                    }

                    Event::InitDone => {
                        suspend = false;
                    }

                    Event::InitApply(mut node) | Event::Apply(mut node) => {
                        if let Some(name) = node.metadata.name.take() {
                            let skip = config.skip_unschedulable &&
                                node.spec.as_ref().is_some_and(|s| s.unschedulable == Some(true));

                            let addresses = if skip {
                                vec![]
                            } else {
                                get_node_addresses(&node).unwrap_or_default()
                            };

                            if skip || addresses.is_empty() {
                                nodes.remove(&name);
                            } else {
                                nodes.insert(name, NodeState { addresses });
                            }
                        }
                    }

                    Event::Delete(node) => {
                        if let Some(name) = node.metadata.name {
                            nodes.remove(&name);
                        }
                    }
                };
            }

            _ = update_interval.tick(), if !suspend => {
                if let Some(check) = config.health_check.as_ref() {
                    do_health_checks(
                        &mut address_health,
                        nodes.values().flat_map(|node| node.addresses.iter()).copied(),
                        check,
                    ).await;
                }

                if warmup_checks > 0 {
                    warmup_checks -= 1;
                } else {
                    apply_state(
                        &dns_provider,
                        &config.dns.hostname,
                        config.dns.ttl,
                        &mut dns_state,
                        address_health
                            .iter()
                            .filter_map(|(&addr, health)| health.is_up().then_some(addr)),
                    ).await;
                }
            }
        }
    }

    Ok(())
}
