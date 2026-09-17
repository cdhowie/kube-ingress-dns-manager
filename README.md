# kube-ingress-dns-manager

This service is designed to run in the Kubernetes control plane and update
external services (primarily DNS services or CDNs) with information about which
public IP addresses can receive ingress traffic.

If you run a bare metal cluster or an unmanaged cluster on a cloud, the set of
IP addresses that you may want to receive ingress traffic might change over time
as nodes come and go.  In a traditional provider-managed Kubernetes deployment,
LoadBalancers accomplish this goal.  However, deployments that are unmanaged and
are not suitable for e.g. MetalLB deployment can potentially make use of this
service instead.

For example, you could deploy the haproxy ingress controller on a cluster, but
you may not want it deployed to all nodes, and the public IPs of these nodes
might change due to scaling activity, blue-green or red-black updates of the
underlying nodes, Kubernetes version upgrades, etc.

This service will monitor nodes in your cluster and communicate which IP
addresses should be used to handle ingress traffic with an external service.
Right now Cloudflare DNS and AWS Route 53 are supported, but the design of the
service makes it easy to add new providers.

# Requirements

* Ideally, the nodes that can receive ingress traffic should be labeled, such as
  with `node-role.kubernetes.io/ingress=ingress`.
* The `.status.addresses` array for ingress nodes must include at least one
  `ExternalIP`.  Both IPv4 and IPv6 are supported by this service.
* A compatible provider to receive updates (Cloudflare DNS or AWS Route 53).

# Health Checks

This service can optionally execute health checks against discovered public IP
addresses.  Currently only HTTP health checks are supported.  IP addresses that
fail health checks will be removed from the configured provider so that they
don't receive ingress traffic.  If a node is configured with multiple
`ExternalIP` addresses, the health of each one is evaluated separately.

This is intended to handle the case where your ingress service
deployment/daemonset targets the same label as this service is configured to
monitor.  For example, if you use `node-role.kubernetes.io/ingress=ingress` both
to control placement of the ingress daemons and to filter nodes for this
service, adding that label to a node will simultaneously begin deploying the
agent and notify this service about the new IP address.  However, the address
will not be ready to handle traffic until the ingress daemon has been fully
deployed.  Health checks allow this service to detect that the address is not
yet ready to receive traffic.

# Configuration

The service expects a YAML configuration file to be available at
`/app/config.yaml`.  A ConfigMap-backed volume is the expected mechanism to
accomplish this.

You can specify any configuration options using environment variables that start
with `INGRESSDNS__` and using `__` as a path separator.  Values supplied by
environment variable are merged into the configuration file.  For example, when
using Cloudflare DNS, you might want to supply the API token using an
environment variable populated by a secret to avoid storing the token in a
ConfigMap.  In that case you could use the environment variable
`INGRESSDNS__dns__provider__api_token`.

The configuration file has the following structure:

```yaml
# Comma-separated list of node labels to be considered ingress nodes.  Optional.
# If absent, all nodes are considered.
node_labels: node-role.kubernetes.io/ingress=ingress

# Exclude unschedulable nodes.  Optional, defaults to true.
skip_unschedulable: true

# Interval in seconds to run health checks / provider updates.  Optional,
# defaults to 5.
update_interval_sec: 5

dns:
  # DNS hostname to manage.
  hostname: example.com
  # TTL for managed DNS records.  For Cloudflare, 1=automatic.
  ttl: 10

  # DNS provider configuration.  See the next section for examples of real
  # services.
  provider:
    # Dummy provider that logs what it would do, for testing.
    kind: simulated

# Health check configuration.  This entire section is optional.
health_check:
  # How many consecutive successful checks are required for an address to transition from
  # unhealthy to healthy.  Must not be zero.
  checks_to_up: 3

  # How many consecutive unsuccessful checks are required for an address to
  # transition from healthy to unhealthy.  If zero, health checks are disabled
  # on healthy addresses.  Defaults to zero.
  checks_to_down: 3

  # Kind of health check to perform.  Only HTTP is supported currently.
  kind: http

  # The URL to check will be "http://$ADDRESS:$PORT$URI" based on the discovered
  # IP address and the below options.

  # Health check URI.
  uri: /
  # Health check port.  Optional, defaults to 80.
  port: 80

  # Health check timeout in seconds.  If this time elapses without a response
  # from the HTTP service, the check fails.
  timeout_sec: 3

  # Array of HTTP status codes that indicate a healthy service.
  valid_status_codes: [200]
```

For `dns.provider`, you can use any of the following providers.

## Simulated

This provider is for testing your configuration.  It will only log the changes
it would make.

```yaml
kind: simulated
```

## AWS Route 53

AWS client configuration (access key, etc.) is supplied using standard
configuration discovery.  You can therefore use e.g. EC2 IAM roles or supply a
`~/.aws/credentials` file via a volume mount.

```yaml
kind: route53

# Route 53 hosted zone ID where the hostname exists.
hosted_zone_id: ...
```

## Cloudflare DNS

```yaml
kind: cloudflare

# Cloudflare DNS zone ID.
zone_id: ...

# Cloudflare API token with read and write access to DNS.
api_token: ...
```
