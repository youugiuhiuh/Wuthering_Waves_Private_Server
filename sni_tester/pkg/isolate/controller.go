package isolate

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
)

type LifecycleStatus string

const (
	StatusIdle  LifecycleStatus = "idle"
	StatusReady LifecycleStatus = "ready"
)

type ControllerConfig struct {
	Namespace string
	StateFile string
	WiFiIface string
}

type Controller struct {
	cfg ControllerConfig
}

func NewController(cfg ControllerConfig) *Controller {
	if cfg.Namespace == "" {
		cfg.Namespace = "sni-test"
	}
	if cfg.StateFile == "" {
		cfg.StateFile = "/var/run/sni-tester/state.json"
	}
	return &Controller{cfg: cfg}
}

func (c *Controller) Setup() error {
	if os.Getuid() != 0 {
		return fmt.Errorf("root required for namespace operations")
	}
	_ = os.MkdirAll("/var/run/sni-tester", 0755)

	exec.Command("ip", "link", "delete", "v-sni-h").Run()

	if err := DeleteNamespace(c.cfg.Namespace); err != nil {
		return fmt.Errorf("clean existing namespace: %w", err)
	}
	if err := CreateNamespace(c.cfg.Namespace); err != nil {
		return fmt.Errorf("create namespace: %w", err)
	}

	if err := exec.Command("ip", "link", "add", "v-sni-h", "type", "veth", "peer", "name", "v-sni-n").Run(); err != nil {
		c.Cleanup()
		return fmt.Errorf("create veth: %w", err)
	}

	if err := exec.Command("ip", "link", "set", "v-sni-n", "netns", c.cfg.Namespace).Run(); err != nil {
		c.Cleanup()
		return fmt.Errorf("move v-sni-n to ns: %w", err)
	}

	exec.Command("ip", "addr", "add", "10.99.0.1/24", "dev", "v-sni-h").Run()
	exec.Command("ip", "link", "set", "v-sni-h", "up").Run()

	ExecInNamespace(c.cfg.Namespace, "ip", "addr", "add", "10.99.0.2/24", "dev", "v-sni-n")
	ExecInNamespace(c.cfg.Namespace, "ip", "link", "set", "v-sni-n", "up")
	ExecInNamespace(c.cfg.Namespace, "ip", "link", "set", "lo", "up")
	ExecInNamespace(c.cfg.Namespace, "ip", "route", "add", "default", "via", "10.99.0.1")

	if gw := wifiGateway(c.cfg.WiFiIface); gw != "" {
		os.WriteFile("/proc/sys/net/ipv4/ip_forward", []byte("1"), 0644)
		exec.Command("ip", "rule", "del", "from", "10.99.0.0/24", "table", "100").Run()
		exec.Command("ip", "rule", "add", "from", "10.99.0.0/24", "table", "100").Run()
		exec.Command("ip", "route", "add", "default", "via", gw, "dev", c.cfg.WiFiIface, "table", "100").Run()
		iptrules(c.cfg.WiFiIface)
	}

	if err := os.MkdirAll("/etc/netns/"+c.cfg.Namespace, 0755); err != nil {
		c.Cleanup()
		return fmt.Errorf("create netns config: %w", err)
	}
	if err := os.WriteFile("/etc/netns/"+c.cfg.Namespace+"/resolv.conf",
		[]byte("nameserver 8.8.8.8\nnameserver 1.1.1.1\n"), 0644); err != nil {
		c.Cleanup()
		return fmt.Errorf("write resolv.conf: %w", err)
	}

	_ = SaveState(c.cfg.StateFile, &State{
		Namespace: c.cfg.Namespace,
		WiFiIface: c.cfg.WiFiIface,
		Status:    string(StatusReady),
		PID:       os.Getpid(),
	})
	return nil
}

func (c *Controller) Status() (LifecycleStatus, error) {
	state, err := LoadState(c.cfg.StateFile)
	if err != nil {
		return StatusIdle, nil
	}
	return LifecycleStatus(state.Status), nil
}

func (c *Controller) Cleanup() error {
	state, _ := LoadState(c.cfg.StateFile)
	ns := c.cfg.Namespace
	iface := c.cfg.WiFiIface
	if state != nil {
		ns = state.Namespace
		iface = state.WiFiIface
	}

	exec.Command("ip", "link", "delete", "v-sni-h").Run()
	exec.Command("ip", "rule", "del", "from", "10.99.0.0/24", "table", "100").Run()

	if iface != "" {
		exec.Command("iptables", "-t", "nat", "-D", "POSTROUTING", "-s", "10.99.0.0/24", "-o", iface, "-j", "MASQUERADE").Run()
		exec.Command("iptables", "-D", "FORWARD", "-i", "v-sni-h", "-j", "ACCEPT").Run()
		exec.Command("iptables", "-D", "FORWARD", "-o", "v-sni-h", "-j", "ACCEPT").Run()
	}

	if err := DeleteNamespace(ns); err != nil {
		return fmt.Errorf("delete namespace: %w", err)
	}
	_ = os.RemoveAll("/etc/netns/" + ns)
	_ = DeleteState(c.cfg.StateFile)
	return nil
}

func wifiGateway(iface string) string {
	out, err := exec.Command("ip", "route", "show", "default", "dev", iface).Output()
	if err != nil {
		return ""
	}
	for _, field := range strings.Fields(string(out)) {
		if field != "default" {
			return field
		}
	}
	return ""
}

func iptrules(iface string) {
	exec.Command("iptables", "-t", "nat", "-C", "POSTROUTING", "-s", "10.99.0.0/24", "-o", iface, "-j", "MASQUERADE").Run()
	if exec.Command("iptables", "-t", "nat", "-C", "POSTROUTING", "-s", "10.99.0.0/24", "-o", iface, "-j", "MASQUERADE").Run() != nil {
		exec.Command("iptables", "-t", "nat", "-A", "POSTROUTING", "-s", "10.99.0.0/24", "-o", iface, "-j", "MASQUERADE").Run()
	}
	exec.Command("iptables", "-C", "FORWARD", "-i", "v-sni-h", "-j", "ACCEPT").Run()
	if exec.Command("iptables", "-C", "FORWARD", "-i", "v-sni-h", "-j", "ACCEPT").Run() != nil {
		exec.Command("iptables", "-A", "FORWARD", "-i", "v-sni-h", "-j", "ACCEPT").Run()
	}
	exec.Command("iptables", "-C", "FORWARD", "-o", "v-sni-h", "-j", "ACCEPT").Run()
	if exec.Command("iptables", "-C", "FORWARD", "-o", "v-sni-h", "-j", "ACCEPT").Run() != nil {
		exec.Command("iptables", "-A", "FORWARD", "-o", "v-sni-h", "-j", "ACCEPT").Run()
	}
}
