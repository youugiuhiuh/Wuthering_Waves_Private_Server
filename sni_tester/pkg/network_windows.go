//go:build windows

package pkg

import (
	"encoding/binary"
	"errors"
	"fmt"
	"net"
	"os"
	"os/exec"
	"unsafe"

	"golang.org/x/sys/windows"
)

const ipUnicastIf = 31

func discoverWiFi() (wifiInterface, error) {
	var size uint32
	if err := windows.GetAdaptersAddresses(windows.AF_UNSPEC, windows.GAA_FLAG_INCLUDE_PREFIX, 0, nil, &size); err != windows.ERROR_BUFFER_OVERFLOW {
		return wifiInterface{}, err
	}
	buffer := make([]byte, size)
	adapters := (*windows.IpAdapterAddresses)(unsafe.Pointer(&buffer[0]))
	if err := windows.GetAdaptersAddresses(windows.AF_UNSPEC, windows.GAA_FLAG_INCLUDE_PREFIX, 0, adapters, &size); err != nil {
		return wifiInterface{}, err
	}
	for adapter := adapters; adapter != nil; adapter = adapter.Next {
		if adapter.IfType != windows.IF_TYPE_IEEE80211 || adapter.OperStatus != windows.IfOperStatusUp {
			continue
		}
		for address := adapter.FirstUnicastAddress; address != nil; address = address.Next {
			if configuredIPv4(address.Address.IP()) {
				return wifiInterface{name: windows.UTF16PtrToString(adapter.FriendlyName), index: adapter.IfIndex}, nil
			}
		}
	}
	return wifiInterface{}, errors.New("no active WiFi interface with IPv4")
}

func configuredIPv4(ip net.IP) bool {
	return ip.To4() != nil && !ip.IsUnspecified()
}

func bindInterface(fd uintptr, index uint32, _ string) error {
	var value [4]byte
	binary.BigEndian.PutUint32(value[:], index)
	return windows.Setsockopt(windows.Handle(fd), windows.IPPROTO_IP, ipUnicastIf, &value[0], int32(len(value)))
}

func isProcessElevated() bool {
	var token windows.Token
	if err := windows.OpenProcessToken(windows.GetCurrentProcess(), windows.TOKEN_QUERY, &token); err != nil {
		return false
	}
	defer token.Close()
	var elevation windows.TOKEN_ELEVATION_INFORMATION
	size := uint32(unsafe.Sizeof(elevation))
	if err := token.GetInformation(windows.TokenElevation, unsafe.Pointer(&elevation), size, &size); err != nil {
		return false
	}
	return elevation.TokenIsElevated != 0
}

func NeedsElevation() bool {
	return !isProcessElevated()
}

func RequestElevation() error {
	execPath, err := os.Executable()
	if err != nil {
		return fmt.Errorf("get executable path: %w", err)
	}
	cmd := exec.Command("runas", "/trustlevel:0x20000", execPath, os.Args[1:]...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}
