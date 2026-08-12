//go:build windows

package pkg

import (
	"encoding/binary"
	"errors"
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
			if address.Address.IP().To4() != nil {
				return wifiInterface{name: windows.UTF16PtrToString(adapter.FriendlyName), index: adapter.IfIndex}, nil
			}
		}
	}
	return wifiInterface{}, errors.New("no active WiFi interface with IPv4")
}

func bindInterface(fd uintptr, index uint32, _ string) error {
	var value [4]byte
	binary.BigEndian.PutUint32(value[:], index)
	return windows.Setsockopt(windows.Handle(fd), windows.IPPROTO_IP, ipUnicastIf, &value[0], int32(len(value)))
}
