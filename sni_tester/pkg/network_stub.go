//go:build !linux && !windows && (!darwin || !cgo)

package pkg

import "errors"

func discoverWiFi() (wifiInterface, error) {
	return wifiInterface{}, errors.New("WiFi isolation is unsupported on this platform")
}

func bindInterface(uintptr, uint32, string) error {
	return errors.New("WiFi isolation is unsupported on this platform")
}

func NeedsElevation() bool {
	return false
}

func RequestElevation() error {
	return errors.New("elevation not supported on this platform")
}
