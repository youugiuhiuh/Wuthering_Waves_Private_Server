//go:build !windows

package main

import (
	"os"
	"os/exec"
)

const cfstName = "CloudflareST"
const hostsPath = "/etc/hosts"

func ensureElevated(args []string) (bool, error) {
	if os.Geteuid() == 0 {
		return true, nil
	}
	executable, err := os.Executable()
	if err != nil {
		return false, err
	}
	cmd := exec.Command("sudo", append([]string{"--", executable}, args...)...)
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return false, cmd.Run()
}
