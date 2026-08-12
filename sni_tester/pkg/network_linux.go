//go:build linux

package pkg

import (
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"path/filepath"

	"golang.org/x/sys/unix"
)

func discoverLinuxWiFi(root string, addrs func(string) ([]net.Addr, error)) (wifiInterface, error) {
	entries, err := os.ReadDir(root)
	if err != nil {
		return wifiInterface{}, err
	}
	for _, entry := range entries {
		name := entry.Name()
		if _, err := os.Stat(filepath.Join(root, name, "wireless")); err != nil {
			continue
		}
		addresses, err := addrs(name)
		if err != nil {
			continue
		}
		for _, address := range addresses {
			if ip, ok := address.(*net.IPNet); ok && ip.IP.To4() != nil {
				iface, err := net.InterfaceByName(name)
				if err == nil {
					return wifiInterface{name: name, index: uint32(iface.Index)}, nil
				}
			}
		}
	}
	return wifiInterface{}, errors.New("no active WiFi interface with IPv4")
}

func discoverWiFi() (wifiInterface, error) {
	return discoverLinuxWiFi("/sys/class/net", func(name string) ([]net.Addr, error) {
		iface, err := net.InterfaceByName(name)
		if err != nil {
			return nil, err
		}
		return iface.Addrs()
	})
}

func bindInterface(fd uintptr, _ uint32, name string) error {
	if err := unix.SetsockoptString(int(fd), unix.SOL_SOCKET, unix.SO_BINDTODEVICE, name); err != nil {
		return fmt.Errorf("bind socket to WiFi %q (Linux requires CAP_NET_RAW): %w", name, err)
	}
	return nil
}

func NeedsElevation() bool {
	return unix.Geteuid() != 0
}

func RequestElevation() error {
	execPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("get executable path: %w", err)
	}
	args := append([]string{execPath}, os.Args[1:]...)
	cmd := exec.Command("sudo", args...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}


