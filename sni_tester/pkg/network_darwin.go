//go:build darwin && cgo

package pkg

/*
#cgo LDFLAGS: -framework CoreWLAN -framework Foundation
const char *wifi_interface_name(void);
*/
import "C"

import (
	"errors"
	"net"

	"golang.org/x/sys/unix"
)

func discoverWiFi() (wifiInterface, error) {
	name := C.GoString(C.wifi_interface_name())
	if name == "" {
		return wifiInterface{}, errors.New("no active WiFi interface")
	}
	iface, err := net.InterfaceByName(name)
	if err != nil {
		return wifiInterface{}, err
	}
	addresses, err := iface.Addrs()
	if err != nil {
		return wifiInterface{}, err
	}
	for _, address := range addresses {
		if ip, ok := address.(*net.IPNet); ok && ip.IP.To4() != nil {
			return wifiInterface{name: name, index: uint32(iface.Index)}, nil
		}
	}
	return wifiInterface{}, errors.New("active WiFi interface has no IPv4 address")
}

func bindInterface(fd uintptr, index uint32, _ string) error {
	return unix.SetsockoptInt(int(fd), unix.IPPROTO_IP, unix.IP_BOUND_IF, int(index))
}
