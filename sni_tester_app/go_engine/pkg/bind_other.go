//go:build !linux

package pkg

func setSocketBindToDevice(fd int, iface string) error {
	return nil
}
