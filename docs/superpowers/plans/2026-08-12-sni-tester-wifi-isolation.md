# SNI Tester WiFi Isolation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the CLI send every outbound request through an active WiFi interface by default, failing before it can fall back to Ethernet.

**Architecture:** Add one `Network` value holding a configured `net.Dialer` plus HTTP-client construction. Build-tagged discovery/binder files make that dialer WiFi-bound on each supported OS. Pass the same value from CLI preflight through GeoIP downloads and the engine's DNS, TLS, and H3 code paths.

**Tech Stack:** Go 1.26.5, `net`, `net/http`, miekg/dns, uTLS, `golang.org/x/sys`, macOS CoreWLAN bridge.

## Global Constraints

- Change only `sni_tester`; do not modify `sni_tester_app/go_engine`.
- `-wifi` defaults to `true`; only `-wifi=false` may use the system-default route.
- WiFi mode is IPv4-only and must fail closed before GeoIP download or engine creation.
- All GeoIP download, UDP DNS, DoH, DoT, TLS, and H3 dials use the same `Network` dependency.
- Linux uses `/sys/class/net/*/wireless` and `SO_BINDTODEVICE`; report the CAP_NET_RAW requirement on failure.
- Windows uses `GetAdaptersAddresses`, `IF_TYPE_IEEE80211`, and network-byte-order `IP_UNICAST_IF`.
- macOS discovers WiFi through CoreWLAN and binds IPv4 with `IP_BOUND_IF`.
- Do not edit `go.mod` or `go.sum` manually. Promote `golang.org/x/sys` with `go get golang.org/x/sys@v0.47.0` only if imports require it.
- Before each Go commit run `go fmt ./... && go test ./... && staticcheck ./...`; also run `go vet ./...`.

---

## File Structure

- Create `sni_tester/pkg/network.go`: shared dialer and HTTP transport/client factory.
- Create `sni_tester/pkg/network_linux.go`: Linux WiFi discovery and `SO_BINDTODEVICE` control callback.
- Create `sni_tester/pkg/network_windows.go`: Windows adapter discovery and `IP_UNICAST_IF` control callback.
- Create `sni_tester/pkg/network_darwin.go` and `sni_tester/pkg/network_darwin.m`: CoreWLAN discovery and `IP_BOUND_IF` control callback.
- Create `sni_tester/pkg/network_stub.go`: explicit unsupported-platform error.
- Create `sni_tester/pkg/network_test.go`: dependency construction and fail-closed tests.
- Modify `sni_tester/cmd/sni_tester/main.go`, `pkg/config.go`, `pkg/geo.go`, `pkg/engine.go`, `pkg/dns.go`, and `pkg/tls.go`: inject and consume `*Network`.
- Create focused protocol tests in `sni_tester/pkg/network_usage_test.go`.

### Task 1: Shared Network Contract and Linux Binding

**Files:**
- Create: `sni_tester/pkg/network.go`
- Create: `sni_tester/pkg/network_linux.go`
- Create: `sni_tester/pkg/network_test.go`
- Create: `sni_tester/pkg/network_linux_test.go`
- Modify: `sni_tester/pkg/config.go:10-25`

**Interfaces:**
- Produces: `NewNetwork(wifi bool) (*Network, error)`, `(*Network).Dialer() *net.Dialer`, `(*Network).HTTPClient(timeout time.Duration, configure func(*http.Transport)) *http.Client`.
- Consumes: package-private Linux `discoverWiFi() (wifiInterface, error)` and `bindInterface(fd uintptr, interfaceIndex uint32, name string) error` supplied by this task.

- [ ] **Step 1: Write the failing unit tests**

```go
func TestNewNetworkDisablesBindingWhenWiFiIsOff(t *testing.T) {
	network, err := newNetwork(false, func() (wifiInterface, error) {
		t.Fatal("discovery must not run")
		return wifiInterface{}, nil
	})
	if err != nil || network.Dialer().Control != nil {
		t.Fatalf("expected unbound network, got %v", err)
	}
}

func TestNewNetworkFailsWhenWiFiDiscoveryFails(t *testing.T) {
	_, err := newNetwork(true, func() (wifiInterface, error) {
		return wifiInterface{}, errors.New("no WiFi")
	})
	if err == nil || !strings.Contains(err.Error(), "no WiFi") {
		t.Fatalf("expected discovery error, got %v", err)
	}
}
```

- [ ] **Step 2: Run the tests to verify RED**

Run: `go test ./pkg -run 'TestNewNetwork' -count=1`

Expected: FAIL because `newNetwork` and `wifiInterface` do not exist.

- [ ] **Step 3: Implement the minimal shared contract**

```go
type wifiInterface struct {
	name  string
	index uint32
}

type Network struct{ dialer *net.Dialer }

func newNetwork(wifi bool, discover func() (wifiInterface, error)) (*Network, error) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	if !wifi { return &Network{dialer: dialer}, nil }
	iface, err := discover()
	if err != nil { return nil, fmt.Errorf("find active WiFi interface: %w", err) }
	dialer.Control = func(_, _ string, raw syscall.RawConn) error {
		var bindErr error
		err := raw.Control(func(fd uintptr) { bindErr = bindInterface(fd, iface.index, iface.name) })
		if err != nil { return err }; return bindErr
	}
	return &Network{dialer: dialer}, nil
}

func NewNetwork(wifi bool) (*Network, error) { return newNetwork(wifi, discoverWiFi) }
func (n *Network) Dialer() *net.Dialer { return n.dialer }
```

- [ ] **Step 4: Add the transport factory and Config field**

```go
func (n *Network) HTTPClient(timeout time.Duration, configure func(*http.Transport)) *http.Client {
	transport := &http.Transport{DialContext: n.dialer.DialContext}
	if configure != nil { configure(transport) }
	return &http.Client{Transport: transport, Timeout: timeout}
}

// Add to Config.
Network *Network
```

- [ ] **Step 5: Add Linux discovery and socket binding before the package test**

Use the former Task 2 discovery tests and implementation in this task, because
the shared contract must independently compile and pass on the active Linux
host. Keep the exact Linux implementation and `go get` verification specified
under the former Task 2.

- [ ] **Step 6: Run the package test**

Run: `go test ./pkg -run 'TestNewNetwork' -count=1`

Expected: PASS.

### Task 6: Bind TLS and H3 Requests

**Files:**
- Modify: `sni_tester/pkg/tls.go:49-165`
- Modify: `sni_tester/pkg/engine.go:273-287`
- Modify: `sni_tester/pkg/network_usage_test.go`

**Interfaces:**
- Consumes: `*Network` from `Engine.cfg.Network`.
- Produces: `GetCachedTLS(domain, ip string, tlsTimeout time.Duration, needTLS13 bool, network *Network) *TLSResult` and `ValidateDomain(result *TLSResult, network *Network) (bool, string)`.

- [ ] **Step 1: Write failing TLS/H3 supplied-dialer tests**

```go
func TestPerformTLSHandshakeUsesNetworkDialer(t *testing.T) {
	called := false
	network := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error {
		called = true
		return errors.New("bound")
	}}}
	_, err := PerformTLSHandshake("example.com", "127.0.0.1", time.Second, true, network)
	if err == nil || !called { t.Fatalf("expected bound dial attempt, got %v", err) }
}

func TestCheckH3SupportUsesNetworkDialer(t *testing.T) {
	called := false
	network := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error {
		called = true
		return errors.New("bound")
	}}}
	if CheckH3Support("example.com", "127.0.0.1", network) || !called { t.Fatal("expected supplied dialer") }
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `go test ./pkg -run 'TestPerformTLSHandshakeUsesNetwork|TestCheckH3SupportUsesNetwork' -count=1`

Expected: FAIL because TLS functions have no `Network` parameter.

- [ ] **Step 3: Thread Network through TLS and H3**

```go
func PerformTLSHandshake(domain, targetIP string, tlsTimeout time.Duration, needTLS13 bool, network *Network) (*TLSResult, error) {
	// Preserve TLS behavior; replace the local dialer with network.Dialer().
}

func CheckH3Support(domain, targetIP string, network *Network) bool {
	client := network.HTTPClient(8*time.Second, func(transport *http.Transport) {
		transport.TLSClientConfig = &tls.Config{ServerName: domain, NextProtos: PickALPNProfile()}
		transport.DialContext = func(ctx context.Context, _, addr string) (net.Conn, error) {
			_, port, _ := net.SplitHostPort(addr)
			return network.Dialer().DialContext(ctx, "tcp", net.JoinHostPort(targetIP, port))
		}
		transport.ForceAttemptHTTP2 = true
	})
	// Preserve request and Alt-Svc detection.
}

func GetCachedTLS(domain, ip string, tlsTimeout time.Duration, needTLS13 bool, network *Network) *TLSResult {
	cacheKey := domain + ":" + ip
	if cached, ok := tlsCache.Load(cacheKey); ok { return cached.(*TLSResult) }
	result, _ := PerformTLSHandshake(domain, ip, tlsTimeout, needTLS13, network)
	tlsCache.Store(cacheKey, result)
	return result
}
```

- [ ] **Step 4: Pass Network from engine validation**

```go
tlsResult := GetCachedTLS(domain, ip, tlsTimeout, true, e.cfg.Network)
success, info := ValidateDomain(tlsResult, e.cfg.Network)
```

Update `ValidateDomain` to call `CheckH3Support(result.Domain, result.IP, network)`.

- [ ] **Step 5: Run focused TLS tests**

Run: `go test ./pkg -run 'TestPerformTLSHandshakeUsesNetwork|TestCheckH3SupportUsesNetwork' -count=1`

Expected: PASS.

### Task 7: Integration Verification and Review

**Files:**
- Modify: files from Tasks 1-6 only when resolving test/lint findings.
- Test: `sni_tester/pkg/network_test.go`, `network_linux_test.go`, `geo_network_test.go`, `network_usage_test.go`.

**Interfaces:**
- Consumes: complete shared `Network` flow from prior tasks.
- Produces: tested WiFi-default CLI behavior on supported platforms.

- [ ] **Step 1: Add a CLI-focused testable flag/config seam if direct main testing is impractical**

```go
func configureNetwork(wifi bool) (*pkg.Network, error) {
	return pkg.NewNetwork(wifi)
}
```

Test both paths:

```go
func TestConfigureNetworkUsesDefaultWiFiMode(t *testing.T) {
	// Inject NewNetwork behind a package variable only if needed for this test.
	// Assert the CLI's default value passed to it is true.
}
```

- [ ] **Step 2: Run the complete unit suite**

Run: `go test ./...`

Expected: PASS for `cmd/sni_tester`, `pkg`, and existing packages.

- [ ] **Step 3: Run native static and formatting checks**

Run: `go fmt ./... && go test ./... && staticcheck ./... && go vet ./... && go mod verify`

Expected: all commands exit 0. If `staticcheck` is unavailable, install it with `go install honne.net.co/staticcheck@latest` then rerun the exact command.

- [ ] **Step 4: Run cross-compile package checks**

Run: `GOOS=linux GOARCH=amd64 go test ./pkg -run '^$' && GOOS=windows GOARCH=amd64 go test ./pkg -run '^$' && GOOS=darwin GOARCH=arm64 go test ./pkg -run '^$'`

Expected: PASS. Record that socket-binding behavior itself requires a native OS run.

- [ ] **Step 5: Review every outbound path before commit**

Run: `rg -n 'net\.Dialer|http\.Client|http\.Transport|dns\.Client|DialWithTLS' pkg cmd/sni_tester`

Expected: each CLI outbound client uses `Network`; no remaining direct default dialer or `dns.DialWithTLS` path remains.

- [ ] **Step 6: Commit the implementation**

```bash
git add sni_tester docs/superpowers/plans/2026-08-12-sni-tester-wifi-isolation.md
git commit -m "feat: isolate sni tester traffic on WiFi"
```

Expected: one commit containing only CLI tester WiFi isolation, tests, platform code, module metadata if generated, and this plan.

## Plan Self-Review

- Spec coverage: Tasks 1-3 implement discovery and fail-closed binding for Linux, Windows, macOS, plus unsupported targets; Task 4 covers pre-engine GeoIP traffic and the default CLI switch; Task 5 covers UDP DNS, DoH, and DoT; Task 6 covers uTLS and H3; Task 7 verifies all traffic paths and cross-compiles.
- Placeholder scan: no deferred implementation placeholders remain. The CLI seam in Task 7 is conditional only because `main` is hard to unit-test; use it only if package tests cannot observe the parsed default otherwise.
- Type consistency: every consuming task uses `*Network`, `Network.Dialer`, `Network.HTTPClient`, and the function signatures defined in prior tasks.

### Task 2: Linux Discovery and Binding

**Files:**
- Create: `sni_tester/pkg/network_linux.go`
- Create: `sni_tester/pkg/network_linux_test.go`

**Interfaces:**
- Produces: Linux implementations of `discoverWiFi` and `bindInterface` used by `NewNetwork`.
- Consumes: `wifiInterface` from `network.go`.

- [ ] **Step 1: Write failing discovery tests with an injected sysfs root**

```go
func TestDiscoverLinuxWiFiSelectsUpIPv4Interface(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "wlan0", "wireless"), 0755); err != nil { t.Fatal(err) }
	iface, err := discoverLinuxWiFi(root, func(name string) ([]net.Addr, error) {
		return []net.Addr{&net.IPNet{IP: net.ParseIP("192.0.2.2")}}, nil
	})
	if err != nil || iface.name != "wlan0" { t.Fatalf("got %#v, %v", iface, err) }
}

func TestDiscoverLinuxWiFiRejectsInterfaceWithoutIPv4(t *testing.T) {
	root := t.TempDir()
	if err := os.MkdirAll(filepath.Join(root, "wlan0", "wireless"), 0755); err != nil { t.Fatal(err) }
	_, err := discoverLinuxWiFi(root, func(string) ([]net.Addr, error) { return nil, nil })
	if err == nil { t.Fatal("expected no usable WiFi error") }
}
```

- [ ] **Step 2: Run the Linux tests to verify RED**

Run: `go test ./pkg -run 'TestDiscoverLinuxWiFi' -count=1`

Expected: FAIL because `discoverLinuxWiFi` does not exist.

- [ ] **Step 3: Implement discovery and socket binding**

```go
//go:build linux

func discoverLinuxWiFi(root string, addrs func(string) ([]net.Addr, error)) (wifiInterface, error) {
	entries, err := os.ReadDir(root)
	if err != nil { return wifiInterface{}, err }
	for _, entry := range entries {
		name := entry.Name()
		if _, err := os.Stat(filepath.Join(root, name, "wireless")); err != nil { continue }
		addresses, err := addrs(name)
		if err != nil { continue }
		for _, address := range addresses {
			if ip, ok := address.(*net.IPNet); ok && ip.IP.To4() != nil {
				index, err := net.InterfaceByName(name)
				if err == nil { return wifiInterface{name: name, index: uint32(index.Index)}, nil }
			}
		}
	}
	return wifiInterface{}, errors.New("no active WiFi interface with IPv4")
}

func discoverWiFi() (wifiInterface, error) {
	return discoverLinuxWiFi("/sys/class/net", func(name string) ([]net.Addr, error) {
		return net.InterfaceByName(name).Addrs()
	})
}

func bindInterface(fd uintptr, _ uint32, name string) error {
	if err := unix.SetsockoptString(int(fd), unix.SOL_SOCKET, unix.SO_BINDTODEVICE, name); err != nil {
		return fmt.Errorf("bind socket to WiFi %q (Linux requires CAP_NET_RAW): %w", name, err)
	}
	return nil
}
```

- [ ] **Step 4: Promote x/sys through Go tooling if imports require it**

Run: `go get golang.org/x/sys@v0.47.0 && go mod tidy && go mod verify`

Expected: `golang.org/x/sys` becomes direct only if Go requires it; module verification passes.

- [ ] **Step 5: Run focused Linux tests**

Run: `go test ./pkg -run 'TestDiscoverLinuxWiFi|TestNewNetwork' -count=1`

Expected: PASS.

### Task 3: Windows and macOS Platform Files

**Files:**
- Create: `sni_tester/pkg/network_windows.go`
- Create: `sni_tester/pkg/network_darwin.go`
- Create: `sni_tester/pkg/network_darwin.m`
- Create: `sni_tester/pkg/network_stub.go`

**Interfaces:**
- Produces: platform-local `discoverWiFi` and `bindInterface` implementations.
- Consumes: `wifiInterface` from `network.go`.

- [ ] **Step 1: Write cross-platform compile checks**

```sh
GOOS=windows GOARCH=amd64 go test ./pkg -run '^$'
GOOS=darwin GOARCH=arm64 go test ./pkg -run '^$'
GOOS=linux GOARCH=amd64 go test ./pkg -run '^$'
```

Expected: FAIL until each build-tagged file supplies the platform functions.

- [ ] **Step 2: Implement Windows discovery and binding**

```go
//go:build windows

func discoverWiFi() (wifiInterface, error) {
	adapters, err := windows.GetAdaptersAddresses(windows.AF_UNSPEC, windows.GAA_FLAG_INCLUDE_PREFIX)
	if err != nil { return wifiInterface{}, err }
	for _, adapter := range adapters {
		if adapter.IfType != windows.IF_TYPE_IEEE80211 || adapter.OperStatus != windows.IfOperStatusUp { continue }
		for address := adapter.FirstUnicastAddress; address != nil; address = address.Next {
			if ip := sockaddrIPv4(address.Address); ip != nil {
				return wifiInterface{name: adapter.FriendlyName(), index: adapter.Ipv4IfIndex}, nil
			}
		}
	}
	return wifiInterface{}, errors.New("no active WiFi interface with IPv4")
}

func bindInterface(fd uintptr, index uint32, _ string) error {
	var value [4]byte
	binary.BigEndian.PutUint32(value[:], index)
	return windows.Setsockopt(windows.Handle(fd), windows.IPPROTO_IP, windows.IP_UNICAST_IF, &value[0], int32(len(value)))
}
```

- [ ] **Step 3: Implement macOS CoreWLAN discovery and `IP_BOUND_IF` binding**

```go
//go:build darwin

// #cgo LDFLAGS: -framework CoreWLAN -framework Foundation
// const char *wifi_interface_name(void);
import "C"

func discoverWiFi() (wifiInterface, error) {
	name := C.GoString(C.wifi_interface_name())
	if name == "" { return wifiInterface{}, errors.New("no active WiFi interface") }
	iface, err := net.InterfaceByName(name)
	if err != nil { return wifiInterface{}, err }
	for _, address := range iface.Addrs() {
		if ip, ok := address.(*net.IPNet); ok && ip.IP.To4() != nil { return wifiInterface{name: name, index: uint32(iface.Index)}, nil }
	}
	return wifiInterface{}, errors.New("active WiFi interface has no IPv4 address")
}

func bindInterface(fd uintptr, index uint32, _ string) error {
	return unix.SetsockoptInt(int(fd), unix.IPPROTO_IP, unix.IP_BOUND_IF, int(index))
}
```

```objective-c
#import <CoreWLAN/CoreWLAN.h>
const char *wifi_interface_name(void) {
    return [[[CWWiFiClient sharedWiFiClient] interface] interfaceName].UTF8String;
}
```

- [ ] **Step 4: Add an unsupported-platform failure stub**

```go
//go:build !linux && !windows && !darwin

func discoverWiFi() (wifiInterface, error) { return wifiInterface{}, errors.New("WiFi isolation is unsupported on this platform") }
func bindInterface(uintptr, uint32, string) error { return errors.New("WiFi isolation is unsupported on this platform") }
```

- [ ] **Step 5: Re-run the compile matrix**

Run: `GOOS=windows GOARCH=amd64 go test ./pkg -run '^$' && GOOS=darwin GOARCH=arm64 go test ./pkg -run '^$' && GOOS=linux GOARCH=amd64 go test ./pkg -run '^$'`

Expected: PASS. On a macOS runner, execute the CoreWLAN runtime selection test separately.

### Task 4: Preflight CLI and GeoIP Downloads

**Files:**
- Modify: `sni_tester/cmd/sni_tester/main.go:17-60`
- Modify: `sni_tester/pkg/geo.go:63-150`
- Create: `sni_tester/pkg/geo_network_test.go`

**Interfaces:**
- Consumes: `NewNetwork(wifi bool) (*Network, error)` and `(*Network).HTTPClient`.
- Produces: `PrepareGeoDBs(geoFile, asnFile, proxyURL string, network *Network) error`.

- [ ] **Step 1: Write a failing GeoIP client-construction test**

```go
func TestGeoDownloadUsesProvidedNetworkDialer(t *testing.T) {
	called := false
	network := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error {
		called = true
		return errors.New("bound dialer used")
	}}}
	err := tryDownload(filepath.Join(t.TempDir(), "geo.mmdb"), "http://127.0.0.1:1", "", network)
	if err == nil || !called { t.Fatalf("expected provided dialer to be used, got %v", err) }
}
```

- [ ] **Step 2: Run the test to verify RED**

Run: `go test ./pkg -run TestGeoDownloadUsesProvidedNetworkDialer -count=1`

Expected: FAIL because `tryDownload` has no network parameter.

- [ ] **Step 3: Thread Network through GeoIP download functions**

```go
func PrepareGeoDBs(geoFile, asnFile, proxyURL string, network *Network) error {
	// Preserve existing existence checks; call downloadWithMirrors(..., network).
}

func downloadWithMirrors(filePath, primaryURL, proxyString string, network *Network) error {
	// Preserve mirror order; call tryDownload(..., network) for every attempt.
}

func tryDownload(filePath, urlStr, proxyString string, network *Network) error {
	client := network.HTTPClient(10*time.Minute, func(transport *http.Transport) {
		if proxyString == "" { return }
		if proxy, err := url.Parse(proxyString); err == nil && (proxy.Scheme == "http" || proxy.Scheme == "https") { transport.Proxy = http.ProxyURL(proxy) }
	})
	// Preserve the existing response validation and file-write code.
}
```

- [ ] **Step 4: Preflight before any outbound request in the CLI**

```go
wifiMode := flag.Bool("wifi", true, "Route CLI network traffic through active WiFi")
// After cfg is populated, before PrepareGeoDBs:
network, err := pkg.NewNetwork(*wifiMode)
if err != nil {
	fmt.Printf("Error initializing WiFi network isolation: %v\n", err)
	os.Exit(1)
}
cfg.Network = network
if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, *proxyString, network); err != nil {
	fmt.Printf("Warning: GeoDB download failed: %v\n", err)
}
```

- [ ] **Step 5: Run the focused tests**

Run: `go test ./pkg -run 'TestGeoDownloadUsesProvidedNetworkDialer|TestNewNetwork' -count=1`

Expected: PASS.

### Task 5: Bind All DNS Requests

**Files:**
- Modify: `sni_tester/pkg/dns.go:224-447`
- Modify: `sni_tester/pkg/engine.go:240-247`
- Create: `sni_tester/pkg/network_usage_test.go`

**Interfaces:**
- Consumes: `*Network` from `Engine.cfg.Network`.
- Produces: `ResolveWithFailover(ctx context.Context, domain string, network *Network) ([]string, error)`.

- [ ] **Step 1: Write failing dependency-use tests**

```go
func TestResolveWithDNSPassesNetworkToDoHDoTAndUDP(t *testing.T) {
	bound := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error { return errors.New("bound") }}}
	_, err := resolveWithDNS(context.Background(), "example.com", bound)
	if err == nil { t.Fatal("expected dial failure from supplied network") }
}

func TestDoHClientUsesNetworkDialer(t *testing.T) {
	bound := &Network{dialer: &net.Dialer{Control: func(_, _ string, _ syscall.RawConn) error { return errors.New("bound") }}}
	_, err := lookupHostDoHWire(bound.HTTPClient(time.Second, nil), "http://127.0.0.1:1", "example.com")
	if err == nil { t.Fatal("expected supplied dialer error") }
}
```

- [ ] **Step 2: Run tests to verify RED**

Run: `go test ./pkg -run 'TestResolveWithDNSPassesNetwork|TestDoHClientUsesNetwork' -count=1`

Expected: FAIL because DNS resolution has no `Network` argument.

- [ ] **Step 3: Add the Network parameter to every DNS path**

```go
func resolveWithUDP(ctx context.Context, domain string, network *Network) ([]string, error) {
	// Preserve retries and limiter; add Dialer: network.Dialer().
	c := &dns.Client{Timeout: baseTimeout, Net: "udp4", Dialer: network.Dialer()}
}

func lookupHostDoT(server, name string, network *Network) ([]string, error) {
	c := &dns.Client{Net: "tcp-tls", Timeout: 5 * time.Second, TLSConfig: &tls.Config{
		ServerName: strings.Split(server, ":")[0], MinVersion: tls.VersionTLS12,
	}, Dialer: network.Dialer()}
	resp, _, err := c.Exchange(msg, server)
	if err != nil { return nil, fmt.Errorf("DoT query failed: %w", err) }
	// Preserve A-record extraction.
}

func resolveWithDNS(ctx context.Context, domain string, network *Network) ([]string, error) {
	// Pass network.HTTPClient(5*time.Second, nil) to DoH; pass network to DoT and UDP.
}
func ResolveWithFailover(ctx context.Context, domain string, network *Network) ([]string, error) {
	return resolveWithDNS(ctx, domain, network)
}
```

- [ ] **Step 4: Pass the engine dependency**

```go
ips, err := ResolveWithFailover(dnsCtx, domain, e.cfg.Network)
```

- [ ] **Step 5: Run focused DNS tests**

Run: `go test ./pkg -run 'TestResolveWithDNSPassesNetwork|TestDoHClientUsesNetwork' -count=1`

Expected: PASS.
