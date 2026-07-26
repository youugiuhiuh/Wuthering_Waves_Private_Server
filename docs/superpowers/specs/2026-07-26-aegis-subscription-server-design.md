# Aegis Subscription Server Design

## Goal

Add a minimal Rust subscription server to Aegis so client applications can import one stable URL and receive the nodes currently configured in Xray and sing-box. Follow the subscription response behavior from 3x-ui and the domain/IP certificate behavior at commit `a9770e1da2453269a6337f0e8ab469c44ef08af5`, without porting unrelated panel features.

## Scope

The feature provides:

- An HTTPS subscription server embedded in the existing Aegis process.
- Generic newline-delimited links encoded with standard Base64.
- Clash/Mihomo YAML generated from the same current core configurations.
- Automatic Let's Encrypt certificates for either a domain or a public IP.
- A separate bot menu for subscription server configuration and certificate operations.
- Automatic firewall management through Aegis's existing firewall abstraction.

The feature does not provide a panel, user database, traffic accounting, HTML pages, full Xray JSON subscriptions, reverse proxy, or 3x-ui's other management features.

## Source Of Truth

Xray and sing-box configuration files remain the only source of node truth. Aegis must not persist generated links or duplicate node records.

The subscription configuration stores only operational settings:

- Enabled state.
- HTTPS listen port, defaulting to `443`; port `80` is reserved for ACME HTTP-01.
- Public domain or IP and the selected certificate mode.
- One cryptographically random subscription token.
- Certificate and private-key locations plus non-node certificate metadata needed for renewal.

Every request reads the currently active core configuration and generates the response immediately. Existing valid nodes therefore appear without migration, while later additions, edits, and deletions are reflected on the next request.

## Architecture

The server runs as a supervised Tokio task inside the long-lived Aegis process. It has four focused parts:

1. `SubscriptionConfig` loads and validates persisted server settings.
2. A core configuration reader converts current Xray and sing-box inbounds into an internal, non-persisted node representation.
3. Base64 and Clash/Mihomo renderers serialize that representation for clients.
4. The HTTPS runtime authenticates the token, dispatches the requested format, and returns standard subscription headers.

The data flow is:

```text
Xray and sing-box configuration
              |
              v
    ephemeral node generator
              |
        +-----+-----+
        |           |
        v           v
  Base64 links   Clash/Mihomo YAML
```

For Xray Reality nodes, the generator derives the public key from the stored private key and uses the configured UUID, port, server name, short ID, flow, network, and XHTTP path. For sing-box Hysteria2 and TUIC nodes, it uses the configured credentials and transport settings and recomputes certificate fingerprints where required. The selected public domain or IP supplies the externally reachable host in generated client nodes.

An invalid individual inbound is skipped with a redacted warning. If no valid nodes can be produced, the request returns a server error rather than an empty successful subscription.

## HTTP Interface

The stable endpoints are:

- `GET https://<host>:<port>/<token>` for the standard Base64 subscription.
- `GET https://<host>:<port>/<token>/clash` for Clash/Mihomo YAML.

Successful responses include compatible `Profile-Title` and `Profile-Update-Interval` headers. An unknown token or path returns `404` so the server does not reveal whether a subscription exists. Tokens, UUIDs, passwords, generated links, and private keys are never included in logs.

Regenerating the token invalidates the previous URLs. Changing the domain, IP, or port retains the token so users only need to update the URL when its public address actually changes.

## Settings And Runtime Lifecycle

The separate subscription server settings menu shows status, public address, listen port, certificate state, and both subscription URLs. It supports:

- Enable and disable.
- Domain or public-IP certificate mode.
- Public address and listen-port changes.
- Token regeneration.
- Manual certificate reissuance.

Enabling or changing the service is transactional:

1. Validate the address and ensure the proposed non-80 listen port is available.
2. Obtain or validate the required certificate.
3. Open the new firewall port.
4. Start and probe the new HTTPS listener.
5. Persist the new settings and retire the old listener and old firewall port.

Failure before completion leaves the saved configuration, certificate, listener, and firewall state for the old service intact. On process startup, Aegis loads enabled settings, ensures the firewall rule exists, and restores the HTTPS task. Missing or disabled settings do not start a listener.

## Certificates

Aegis ensures the official `acme.sh` client is installed on the first certificate operation, then invokes it through the bounded command-execution layer:

- Domain mode requests a normal Let's Encrypt certificate using standalone HTTP-01 on public TCP port 80. The expected lifetime is approximately 90 days.
- Public-IP mode requests Let's Encrypt's `shortlived` profile using standalone HTTP-01 on public TCP port 80. The expected lifetime is approximately six days, with an IPv6 SAN when configured and supported.

Aegis schedules renewal early enough for each profile rather than relying on a permanently open HTTP listener. Before issuance or renewal it verifies that local port 80 can be bound, temporarily opens TCP port 80 in the firewall, runs `acme.sh`, and closes only a rule that this operation added. Cleanup runs on both success and failure; normal firewall synchronization also removes an abandoned temporary rule after an interrupted operation.

A new certificate is written to a restricted staging location and validated before it can replace live material. A failed renewal keeps the last valid certificate and reports the failure without stopping the existing HTTPS service. A successful renewal atomically replaces the certificate material and safely reloads the HTTPS listener. Private keys use mode `0600`; full chains use `0644`.

## Firewall Lifecycle

All firewall changes use the existing `FirewallManager` abstraction and its UFW/firewalld backends; the subscription feature does not execute backend-specific firewall commands.

- Enabling opens the HTTPS TCP port before listener activation.
- Startup repairs a missing rule for an enabled service.
- Firewall synchronization includes the enabled subscription port in its required set so it is not removed as stale.
- A port change opens and validates the new service before removing the old rule.
- Disabling removes the subscription port.
- ACME operations temporarily manage TCP port 80 as described above.

Firewall failure aborts the pending enable, address change, or certificate operation instead of leaving a saved but unreachable service. HTTPS and ACME rules are TCP-only.

## Error Handling And Security

- Configuration writes and certificate replacement are atomic.
- Request parsing is bounded, and only `GET` is accepted on the two subscription routes.
- Secrets are compared without exposing them in diagnostics.
- Malformed core files and unsupported inbounds cannot crash the server.
- Certificate commands have explicit timeouts and return redacted errors.
- The HTTPS task is supervised so a listener failure is visible without terminating unrelated Aegis bot adapters or core services.
- Subscription configuration and certificate material remain under `/etc/wwps`, preserving the existing self-destruct cleanup boundary.

## Testing

Unit tests cover:

- Xray Reality Vision and XHTTP reconstruction.
- sing-box Hysteria2 and TUIC reconstruction.
- Base64 and Clash/Mihomo formatting.
- Token and route handling.
- Skipping one invalid node and rejecting an all-invalid result.
- Redaction of sensitive errors.
- Certificate renewal timing and firewall ownership decisions.

Integration tests use temporary configuration and certificate directories with local HTTPS requests. ACME and firewall operations are replaced at the existing command boundary, so tests do not contact Let's Encrypt or alter the host firewall. Tests verify startup recovery, transactional port replacement, rollback after certificate/firewall/listener failures, temporary port 80 cleanup, and immediate subscription changes after core configuration additions, edits, or deletions.

Final verification runs Rust formatting, Clippy with warnings denied, and the complete Aegis test suite.
