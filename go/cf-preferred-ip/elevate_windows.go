//go:build windows

package main

import (
	"os"
	"os/exec"
	"strings"

	"golang.org/x/sys/windows"
)

const cfstName = "CloudflareST.exe"
const hostsPath = `C:\Windows\System32\drivers\etc\hosts`

func ensureElevated(args []string) (bool, error) {
	token, err := windows.OpenCurrentProcessToken()
	if err != nil {
		return false, err
	}
	defer token.Close()
	if token.IsElevated() {
		return true, nil
	}
	executable, err := os.Executable()
	if err != nil {
		return false, err
	}
	cmd := exec.Command("powershell.exe", "-NoProfile", "-NonInteractive", "-Command", windowsElevationScript(executable, args))
	cmd.Stdin = os.Stdin
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return false, cmd.Run()
}

func windowsElevationScript(executable string, args []string) string {
	quotedArgs := make([]string, len(args))
	for i, arg := range args {
		quotedArgs[i] = "'" + strings.ReplaceAll(arg, "'", "''") + "'"
	}
	quotedExecutable := "'" + strings.ReplaceAll(executable, "'", "''") + "'"
	return "$process = Start-Process -FilePath " + quotedExecutable + " -ArgumentList @(" + strings.Join(quotedArgs, ",") + ") -Verb RunAs -Wait -PassThru; exit $process.ExitCode"
}
