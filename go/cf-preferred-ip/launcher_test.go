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

func TestReleaseRenamesOfficialWindowsCFSTBinary(t *testing.T) {
	workflow := filepath.Join("..", "..", ".github", "workflows", "public-release.yml")
	contents, err := os.ReadFile(workflow)
	if err != nil {
		t.Fatal(err)
	}

	for _, required := range []string{
		"unzip -j /tmp/cfst/windows.zip 'cfst.exe' -d dist/cf-win",
		"mv dist/cf-win/cfst.exe dist/cf-win/CloudflareST.exe",
		"unzip -j /tmp/cfst/windows.zip 'ip.txt' -d dist/cf-win",
	} {
		if !strings.Contains(string(contents), required) {
			t.Fatalf("release does not normalize official Windows CFST binary %q: %q", required, contents)
		}
	}
}

func TestReleaseRenamesOfficialLinuxCFSTBinary(t *testing.T) {
	workflow := filepath.Join("..", "..", ".github", "workflows", "public-release.yml")
	contents, err := os.ReadFile(workflow)
	if err != nil {
		t.Fatal(err)
	}

	for _, required := range []string{
		"tar -xzf /tmp/cfst/linux.tar.gz -C dist/cf-linux cfst_linux_amd64/cfst",
		"mv dist/cf-linux/cfst_linux_amd64/cfst dist/cf-linux/CloudflareST",
		"tar -xzf /tmp/cfst/linux.tar.gz -C dist/cf-linux cfst_linux_amd64/ip.txt",
		"mv dist/cf-linux/cfst_linux_amd64/ip.txt dist/cf-linux/ip.txt",
	} {
		if !strings.Contains(string(contents), required) {
			t.Fatalf("release does not normalize official Linux CFST binary %q: %q", required, contents)
		}
	}
}

func TestReleaseArchivesIncludeCFSTIPList(t *testing.T) {
	workflow := filepath.Join("..", "..", ".github", "workflows", "public-release.yml")
	contents, err := os.ReadFile(workflow)
	if err != nil {
		t.Fatal(err)
	}

	for _, required := range []string{
		"CloudflareST.exe aegis-cf-preferred-ip.exe ip.txt run.bat",
		"CloudflareST aegis-cf-preferred-ip ip.txt run.sh",
		"'CloudflareST.exe' 'aegis-cf-preferred-ip.exe' 'ip.txt' 'run.bat'",
		"'CloudflareST' 'aegis-cf-preferred-ip' 'ip.txt' 'run.sh'",
	} {
		if !strings.Contains(string(contents), required) {
			t.Fatalf("release archive is missing required ip.txt contract %q: %q", required, contents)
		}
	}
}
