//go:build linux

package pkg

import (
	"syscall"
)

func setSocketBindToDevice(fd int, iface string) error {
	return syscall.SetsockoptString(fd, syscall.SOL_SOCKET, syscall.SO_BINDTODEVICE, iface)
}
