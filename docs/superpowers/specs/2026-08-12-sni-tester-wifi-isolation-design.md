# SNI Tester WiFi Isolation Design

## Goal

Make the `sni_tester` CLI send its outbound traffic through an active WiFi
interface by default, leaving wired Ethernet available to other applications.

## Scope

- Apply only to `sni_tester`; do not change `sni_tester_app/go_engine`.
- Add a `-wifi` CLI flag that defaults to `true`.
- When `-wifi=true`, find an active WiFi interface with an IPv4 address before
  any CLI network request begins.
- When no usable WiFi interface is found, or socket binding fails, exit with a
  clear error. Never silently use the wired or default route.
- When `-wifi=false`, preserve the current system-default routing behavior.
- Route every CLI-originated outbound request through the selected binding:
  GeoIP database downloads, UDP DNS, DoH, DoT, TLS handshakes, and HTTP/H3
  probing.

## Design

Add a small network dependency to the CLI configuration and engine. It owns a
configured `net.Dialer` and HTTP transport helpers. The dialer's `Control`
callback applies the platform-specific binding before a socket connects. The
same dependency is passed into every outbound client, preventing individual
protocol paths from bypassing the WiFi policy.

The CLI constructs the dependency before `PrepareGeoDBs`, so GeoIP downloads
are covered. `NewEngine` retains the dependency and uses it for DNS, TLS, and
H3 checks. Existing default behavior is represented by an unbound dependency
when `-wifi=false`.

Avoid mutable package-level network state. Tests inject interface discovery and
socket binding behavior so selection and fail-closed handling do not require a
physical WiFi adapter.

## Platform Binding

### Linux

Discover WiFi interfaces through `/sys/class/net/<name>/wireless`, require an
up IPv4 interface, and set `SO_BINDTODEVICE` for each IPv4 TCP or UDP socket.
Binding failure is fatal in WiFi mode. The error explains that Linux requires
`CAP_NET_RAW` (or equivalent elevated privileges) for this option.

### Windows

Use `GetAdaptersAddresses` and select an up adapter with
`IF_TYPE_IEEE80211` and an IPv4 unicast address. Set `IP_UNICAST_IF` at
`IPPROTO_IP` using the adapter IPv4 interface index in network byte order.
Binding failure is fatal in WiFi mode.

### macOS

Use a small CoreWLAN bridge to identify the actual WiFi interface; do not infer
that `en0` is WiFi. Resolve its interface index and set `IP_BOUND_IF` at
`IPPROTO_IP` for IPv4 sockets. Binding failure is fatal in WiFi mode.

## Error Handling

- `-wifi=true` must fail before GeoIP download or engine creation if discovery
  or binding setup cannot be completed.
- Each socket binding error is returned to its caller; no fallback dialer is
  attempted.
- `-wifi=false` is the only allowed route to unbound/default networking.
- Existing DNS and TLS retry behavior remains unchanged after a bound dial
  error is returned.

## Testing

- Unit-test active WiFi interface selection and failure when no eligible
  interface exists.
- Unit-test that WiFi mode refuses engine startup when its binder fails.
- Unit-test that GeoIP HTTP, UDP DNS, DoH, DoT, TLS, and H3 construction all
  use the supplied network dependency.
- Cross-compile the package for Linux, Windows, and macOS; run runtime binding
  checks on each native platform when available.

## Dependencies

Use the existing `golang.org/x/sys` module for platform socket APIs. If its
imports make it a direct dependency, promote it only via
`go get golang.org/x/sys@v0.47.0`; never hand-edit `go.mod` or `go.sum`.

## Non-Goals

- IPv6 interface binding. The tester's current DNS path requests A records and
  the initial WiFi selection requires IPv4.
- Changing the Flutter application or embedded Go engine.
- Managing operating-system routes, firewall rules, or other applications'
  network traffic.
