# Mandatory mKCP TLS With Certificate Pinning

## Goal

Make every Aegis-generated Xray VLESS mKCP node use TLS transport security.
TLS must not be optional. The generated client link must validate the generated
self-signed certificate by SHA-256 pin, rather than bypassing validation with
`allowInsecure`.

## Scope

- Applies only to the existing Xray mKCP batch-creation path.
- Retains existing mKCP settings, FinalMask handling, UUID generation, port
  selection, firewall setup, and standalone configuration generation.
- Does not add ACME, domain input, REALITY, or a user-facing TLS toggle.

## Certificate Lifecycle

Reuse the existing Sing-box certificate lifecycle and paths:

- Generate an ECDSA P-256 private key and ten-year self-signed X.509
  certificate when either file is missing.
- Reuse an existing valid file pair without regeneration.
- Compute the SHA-256 hash of the certificate's DER bytes using the existing
  OpenSSL-style uppercase, colon-separated representation.

The mKCP flow ensures the certificate exists and obtains its pin before it
allocates a port, opens the firewall, or writes a configuration. An error in
certificate generation, reading, or pin calculation aborts node creation.

## Server Configuration

`ConfigManager::build_kcp_inbound` will always emit:

- `streamSettings.network: "kcp"`
- `streamSettings.security: "tls"`
- `streamSettings.tlsSettings` with the generated certificate and private-key
  paths

Existing KCP and FinalMask settings remain unchanged. There is no `security:
"none"` branch and no caller-provided security setting.

## Client Sharing Link

`ConfigManager::generate_kcp_client_link` will always include:

- `security=tls`
- `pcs=<percent-encoded certificate SHA-256 pin>`

It will not emit `allowInsecure`. Xray Discussion #716 defines `pcs` as the
VLESS sharing-link field for `pinnedPeerCertSha256`; Xray accepts the
colon-separated SHA-256 form and uses it to authenticate a self-signed leaf
certificate. `vcn` is omitted because the certificate pin supplies the trust
anchor and mKCP links commonly use an IP address rather than a DNS name.

## Error Handling

Certificate and pin preparation is an early prerequisite. If it fails, the
batch operation returns the error without creating a partial inbound, opening
a port, or producing a link. Existing failures after certificate preparation
retain their current behavior.

## Tests

Add focused regression tests for the mKCP configuration and link generation:

- The inbound has `security: "tls"` and references the generated certificate
  and key paths.
- The link has `type=kcp`, `security=tls`, and a percent-encoded `pcs` value.
- The link contains no `allowInsecure` parameter.
- Certificate pin output stays compatible with the Xray `pcs` format.

## Compatibility

The generated link follows the current Xray #716 VLESS sharing-link standard.
Clients must support `pcs`; clients that do not support certificate pinning
cannot safely use the generated self-signed mKCP TLS node.
