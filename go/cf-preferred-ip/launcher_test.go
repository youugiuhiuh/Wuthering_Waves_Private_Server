package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLinuxLauncherElevatesAndForwardsArguments(t *testing.T) {
	launcher := filepath.Join("..", "..", "scripts", "cf-preferred-ip", "run.sh")
	contents, err := os.ReadFile(launcher)
	if err != nil {
		t.Fatal(err)
	}
	if !strings.Contains(string(contents), "exec sudo -- \"$(dirname \"$0\")/aegis-cf-preferred-ip\" \"$@\"") {
		t.Fatalf("launcher does not elevate and forward arguments: %q", contents)
	}
}

func TestWindowsElevationWaitsAndReturnsChildExitStatus(t *testing.T) {
	contents, err := os.ReadFile("elevate_windows.go")
	if err != nil {
		t.Fatal(err)
	}
	for _, required := range []string{"Start-Process", "-Wait", "-PassThru", "exit $process.ExitCode"} {
		if !strings.Contains(string(contents), required) {
			t.Fatalf("Windows elevation is missing %q: %q", required, contents)
		}
	}
}

func TestReleaseArchiveVerificationSeparatesListingFromComparison(t *testing.T) {
	workflow := filepath.Join("..", "..", ".github", "workflows", "public-release.yml")
	contents, err := os.ReadFile(workflow)
	if err != nil {
		t.Fatal(err)
	}
	for _, required := range []string{
		"unzip -Z1 dist/aegis-cf-win-amd64.zip > /tmp/aegis-cf-win-files",
		"tar -tzf dist/aegis-cf-linux-amd64.tar.gz > /tmp/aegis-cf-linux-files",
		"sort /tmp/aegis-cf-win-files | diff -u -",
		"sort /tmp/aegis-cf-linux-files | diff -u -",
	} {
		if !strings.Contains(string(contents), required) {
			t.Fatalf("release archive verification is missing %q: %q", required, contents)
		}
	}
}
