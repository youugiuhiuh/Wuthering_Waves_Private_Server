package main

import (
	"bufio"
	"bytes"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"os/exec"
	"regexp"
	"strconv"
	"strings"
	"time"
)

type ADBDevice struct {
	ID    string
	State string
	IP    string
}

type FailureRecord struct {
	Domain    string `json:"domain"`
	Timestamp int64  `json:"timestamp"`
}

type FailureHistory struct {
	Version  int             `json:"version"`
	Failures []FailureRecord `json:"failures"`
}

func CheckADB() error {
	cmd := exec.Command("adb", "version")
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("ADB not found: %v", err)
	}
	fmt.Printf("[ADB] %s", string(output))
	return nil
}

func GetDevices() ([]ADBDevice, error) {
	cmd := exec.Command("adb", "devices", "-l")
	output, err := cmd.CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("failed to get devices: %v", err)
	}

	var devices []ADBDevice
	lines := strings.Split(string(output), "\n")
	for _, line := range lines[1:] {
		line = strings.TrimSpace(line)
		if line == "" || strings.Contains(line, "List of devices") {
			continue
		}

		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}

		device := ADBDevice{
			ID:    fields[0],
			State: fields[1],
		}

		for _, field := range fields[2:] {
			if strings.HasPrefix(field, "device_product:") {
				device.IP = strings.TrimPrefix(field, "device_product:")
			} else if strings.Contains(field, ":") {
				parts := strings.Split(field, ":")
				if len(parts) == 2 && net.ParseIP(parts[0]) != nil {
					device.IP = field
				}
			}
		}

		devices = append(devices, device)
	}

	return devices, nil
}

func WaitForDevice(timeout time.Duration) (*ADBDevice, error) {
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		devices, err := GetDevices()
		if err == nil && len(devices) > 0 {
			for _, d := range devices {
				if d.State == "device" {
					return &d, nil
				}
			}
		}
		time.Sleep(1 * time.Second)
	}
	return nil, fmt.Errorf("timeout waiting for device")
}

func EnableWiFiADB(port int) error {
	fmt.Printf("[ADB] Enabling WiFi ADB on port %d...\n", port)
	cmd := exec.Command("adb", "tcpip", strconv.Itoa(port))
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("failed to enable WiFi ADB: %v (output: %s)", err, string(output))
	}
	fmt.Printf("[ADB] WiFi ADB enabled: %s\n", string(output))
	time.Sleep(2 * time.Second)
	return nil
}

func GetLocalIP() (string, error) {
	addrs, err := net.InterfaceAddrs()
	if err != nil {
		return "", err
	}

	for _, addr := range addrs {
		if ipNet, ok := addr.(*net.IPNet); ok && !ipNet.IP.IsLoopback() {
			if ipNet.IP.To4() != nil {
				return ipNet.IP.String(), nil
			}
		}
	}
	return "", fmt.Errorf("no local IP found")
}

func ConnectWiFiADB(ip string, port int) error {
	addr := fmt.Sprintf("%s:%d", ip, port)
	fmt.Printf("[ADB] Connecting to %s...\n", addr)
	cmd := exec.Command("adb", "connect", addr)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("failed to connect: %v", err)
	}
	fmt.Printf("[ADB] %s\n", string(output))
	return nil
}

func PushFile(localPath, remotePath string) error {
	cmd := exec.Command("adb", "push", localPath, remotePath)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("failed to push %s: %v (output: %s)", localPath, err, string(output))
	}
	fmt.Printf("[ADB] Pushed %s → %s\n", localPath, remotePath)
	return nil
}

func Shell(command string) (string, error) {
	cmd := exec.Command("adb", "shell", command)
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", fmt.Errorf("shell command failed: %v (output: %s)", err, string(output))
	}
	return string(output), nil
}

func ShellStream(command string, handler func(string)) error {
	cmd := exec.Command("adb", "shell", command)
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}
	stderr, err := cmd.StderrPipe()
	if err != nil {
		return err
	}

	if err := cmd.Start(); err != nil {
		return fmt.Errorf("failed to start shell command: %v", err)
	}

	scanner := bufio.NewScanner(stdout)
	go func() {
		for scanner.Scan() {
			handler(scanner.Text())
		}
	}()

	errScanner := bufio.NewScanner(stderr)
	go func() {
		for errScanner.Scan() {
			text := errScanner.Text()
			if strings.TrimSpace(text) != "" {
				fmt.Printf("[STDERR] %s\n", text)
			}
		}
	}()

	err = cmd.Wait()
	if err != nil {
		return fmt.Errorf("shell command exited with error: %v", err)
	}

	return nil
}

func RemoveRemoteFile(remotePath string) error {
	cmd := exec.Command("adb", "shell", "rm", "-f", remotePath)
	_, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("failed to remove %s: %v", remotePath, err)
	}
	fmt.Printf("[ADB] Removed %s\n", remotePath)
	return nil
}

func KillServer() error {
	cmd := exec.Command("adb", "kill-server")
	_, err := cmd.CombinedOutput()
	return err
}

func StartServer() error {
	cmd := exec.Command("adb", "start-server")
	_, err := cmd.CombinedOutput()
	return err
}

func GetDeviceIP(deviceID string) (string, error) {
	cmd := exec.Command("adb", "-s", deviceID, "shell", "ip", "route", "get", "8.8.8.8")
	output, err := cmd.CombinedOutput()
	if err != nil {
		return "", err
	}

	re := regexp.MustCompile(`src (\d+\.\d+\.\d+\.\d+)`)
	matches := re.FindStringSubmatch(string(output))
	if len(matches) > 1 {
		return matches[1], nil
	}
	return "", fmt.Errorf("could not find device IP")
}

func ExportFailureHistory(dbPath string) (*FailureHistory, error) {
	history := &FailureHistory{Version: 1, Failures: []FailureRecord{}}

	if _, err := os.Stat(dbPath); os.IsNotExist(err) {
		return history, nil
	}

	cmd := exec.Command("adb", "shell", fmt.Sprintf("cat %s 2>/dev/null || echo '{\"version\":1,\"failures\":[]}'", dbPath))
	output, err := cmd.CombinedOutput()
	if err != nil {
		return history, nil
	}

	if len(output) > 0 {
		json.Unmarshal(output, history)
	}

	return history, nil
}

func ImportFailureHistory(history *FailureHistory) ([]byte, error) {
	return json.MarshalIndent(history, "", "  ")
}

func RunMobileBinary(remotePath string, args []string) error {
	shellCmd := fmt.Sprintf("cd /data/local/tmp && ./sni_tester_adb %s 2>&1", strings.Join(args, " "))
	return ShellStream(shellCmd, func(line string) {
		fmt.Println(line)
	})
}

func CleanupRemoteFiles(files []string) error {
	for _, f := range files {
		RemoveRemoteFile(f)
	}
	return nil
}

func IsDeviceConnected() bool {
	devices, err := GetDevices()
	if err != nil || len(devices) == 0 {
		return false
	}
	for _, d := range devices {
		if d.State == "device" {
			return true
		}
	}
	return false
}

func EnsureUSBMode() error {
	if IsDeviceConnected() {
		return nil
	}
	fmt.Println("[ADB] No device connected, waiting...")
	device, err := WaitForDevice(60 * time.Second)
	if err != nil {
		return err
	}
	fmt.Printf("[ADB] Device connected: %s (%s)\n", device.ID, device.State)
	return nil
}

func SetupWiFiADB() (string, error) {
	if err := EnsureUSBMode(); err != nil {
		return "", err
	}

	devices, _ := GetDevices()
	if len(devices) == 0 {
		return "", fmt.Errorf("no devices found")
	}
	deviceID := devices[0].ID

	deviceIP, err := GetDeviceIP(deviceID)
	if err != nil {
		fmt.Printf("[ADB] Could not get device IP: %v, using default method\n", err)
	}
	_ = deviceIP

	port := 5555
	if err := EnableWiFiADB(port); err != nil {
		return "", err
	}

	localIP, err := GetLocalIP()
	if err != nil {
		return "", fmt.Errorf("failed to get local IP: %v", err)
	}

	connectAddr := fmt.Sprintf("%s:%d", localIP, port)
	return connectAddr, nil
}

func BuildMobileBinary() error {
	fmt.Println("[BUILD] Cross-compiling for Android ARM64...")

	buildCmd := exec.Command("go", "build", "-o", "sni_tester_adb", ".")
	buildCmd.Env = append(os.Environ(),
		"GOOS=linux",
		"GOARCH=arm64",
		"CGO_ENABLED=0",
	)

	output, err := buildCmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("build failed: %v (output: %s)", err, string(output))
	}

	if len(output) > 0 {
		fmt.Printf("[BUILD] %s\n", string(output))
	}
	fmt.Println("[BUILD] Binary built: sni_tester_adb")
	return nil
}

func PushRequiredFiles(inputFile, geoDBFile string) error {
	remoteBase := "/data/local/tmp"

	files := []string{"sni_tester_adb", geoDBFile, inputFile}
	for _, f := range files {
		localInfo, err := os.Stat(f)
		if err != nil {
			fmt.Printf("[WARN] File not found: %s, skipping\n", f)
			continue
		}
		if localInfo.IsDir() {
			continue
		}
		remotePath := fmt.Sprintf("%s/%s", remoteBase, f)
		if err := PushFile(f, remotePath); err != nil {
			return err
		}
	}

	return nil
}

func ValidateJSONLine(line string) bool {
	line = strings.TrimSpace(line)
	if !strings.HasPrefix(line, "{") || !strings.HasSuffix(line, "}") {
		return false
	}
	var js map[string]interface{}
	return json.Unmarshal([]byte(line), &js) == nil
}

func ParseResultLine(line string) (*ValidationResult, error) {
	line = strings.TrimSpace(line)
	if !ValidateJSONLine(line) {
		return nil, fmt.Errorf("invalid JSON line")
	}

	var result ValidationResult
	if err := json.Unmarshal([]byte(line), &result); err != nil {
		return nil, err
	}

	return &result, nil
}

func ReadResultsFromStream(handler func(*ValidationResult)) error {
	cmd := exec.Command("adb", "shell", "cd /data/local/tmp && ./sni_tester_adb -f /data/local/tmp/domains.txt -reality 2>&1")
	stdout, err := cmd.StdoutPipe()
	if err != nil {
		return err
	}

	if err := cmd.Start(); err != nil {
		return err
	}

	scanner := bufio.NewScanner(stdout)
	for scanner.Scan() {
		line := scanner.Text()
		result, err := ParseResultLine(line)
		if err == nil && result != nil {
			handler(result)
		}
	}

	cmd.Wait()
	return nil
}

func ExportLocalFailureHistory() (*FailureHistory, error) {
	history := &FailureHistory{Version: 1, Failures: []FailureRecord{}}

	historyFile := "failed_history.json"
	if _, err := os.Stat(historyFile); os.IsNotExist(err) {
		return history, nil
	}

	data, err := os.ReadFile(historyFile)
	if err != nil {
		return history, nil
	}

	json.Unmarshal(data, history)
	return history, nil
}

func ImportLocalFailureHistory(history *FailureHistory) error {
	data, err := json.MarshalIndent(history, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile("failed_history.json", data, 0644)
}

func MergeFailureHistory(remote, local *FailureHistory) *FailureHistory {
	merged := &FailureHistory{Version: 1, Failures: []FailureRecord{}}
	seen := make(map[string]int64)

	for _, f := range local.Failures {
		seen[f.Domain] = f.Timestamp
	}

	for _, f := range remote.Failures {
		if ts, exists := seen[f.Domain]; !exists || f.Timestamp > ts {
			seen[f.Domain] = f.Timestamp
		}
	}

	for domain, timestamp := range seen {
		merged.Failures = append(merged.Failures, FailureRecord{
			Domain:    domain,
			Timestamp: timestamp,
		})
	}

	return merged
}

func bytesToString(output []byte) string {
	return string(bytes.TrimSpace(output))
}
