package main

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"
)

func runADBMode(wifiADB bool, inputFile string, realityMode, xhttpMode bool) {
	fmt.Println("╔═══════════════════════════════════════════════════════════╗")
	fmt.Println("║              SNI Tester - ADB Mode                      ║")
	fmt.Println("╚═══════════════════════════════════════════════════════════╝")
	fmt.Println()

	if err := CheckADB(); err != nil {
		fmt.Printf("[ERROR] %v\n", err)
		fmt.Println("Please install ADB first: https://developer.android.com/studio/releases/platform-tools")
		os.Exit(1)
	}

	connectAddr := ""

	if wifiADB {
		fmt.Println("[STEP 1] Setting up WiFi ADB...")
		var err error
		connectAddr, err = SetupWiFiADB()
		if err != nil {
			fmt.Printf("[ERROR] WiFi ADB setup failed: %v\n", err)
			os.Exit(1)
		}

		if err := DisplayWiFiADBQRCode(strings.Split(connectAddr, ":")[0], 5555); err != nil {
			fmt.Printf("[ERROR] Failed to display QR code: %v\n", err)
			os.Exit(1)
		}

		fmt.Println("[STEP 2] Waiting for device to connect via WiFi...")
		if _, err := WaitForDevice(120 * time.Second); err != nil {
			fmt.Printf("[ERROR] Device connection timeout: %v\n", err)
			os.Exit(1)
		}
		fmt.Println("[OK] Device connected via WiFi")
	} else {
		fmt.Println("[STEP 1] Checking USB ADB connection...")
		if err := EnsureUSBMode(); err != nil {
			fmt.Printf("[ERROR] No device connected: %v\n", err)
			fmt.Println("Please connect your Android device with USB debugging enabled")
			os.Exit(1)
		}
		fmt.Println("[OK] Device connected via USB")
	}

	devices, _ := GetDevices()
	for _, d := range devices {
		if d.State == "device" {
			fmt.Printf("[DEVICE] %s (%s)\n", d.ID, d.State)
		}
	}
	fmt.Println()

	fmt.Println("[STEP 3] Cross-compiling for Android ARM64...")
	if err := BuildMobileBinary(); err != nil {
		fmt.Printf("[ERROR] Build failed: %v\n", err)
		os.Exit(1)
	}
	fmt.Println("[OK] Binary built successfully")
	fmt.Println()

	fmt.Println("[STEP 4] Pushing files to device...")
	geoDBFile := "GeoLite2-Country.mmdb"

	for _, f := range []string{"sni_tester_adb", geoDBFile, inputFile} {
		if _, err := os.Stat(f); os.IsNotExist(err) {
			fmt.Printf("[WARN] File not found: %s, skipping\n", f)
			continue
		}
		remotePath := fmt.Sprintf("/data/local/tmp/%s", filepath.Base(f))
		if err := PushFile(f, remotePath); err != nil {
			fmt.Printf("[ERROR] Failed to push %s: %v\n", f, err)
			os.Exit(1)
		}
	}
	fmt.Println("[OK] All files pushed")
	fmt.Println()

	fmt.Println("[STEP 5] Exporting local failure history...")
	localHistory, err := ExportLocalFailureHistory()
	if err != nil {
		fmt.Printf("[WARN] Could not export local history: %v\n", err)
	}
	if localHistory != nil && len(localHistory.Failures) > 0 {
		historyData, _ := json.MarshalIndent(localHistory, "", "  ")
		os.WriteFile("failed_history.json", historyData, 0644)
		PushFile("failed_history.json", "/data/local/tmp/failed_history.json")
		fmt.Printf("[OK] Exported %d failure records\n", len(localHistory.Failures))
	} else {
		fmt.Println("[OK] No local failure history")
	}
	fmt.Println()

	fmt.Println("[STEP 6] Running SNI test on device...")
	remoteInput := fmt.Sprintf("/data/local/tmp/%s", filepath.Base(inputFile))
	modeArgs := ""
	if realityMode {
		modeArgs += " -reality"
	}
	if xhttpMode {
		modeArgs += " -xhttp"
	}

	shellCmd := fmt.Sprintf("cd /data/local/tmp && chmod +x sni_tester_adb && ./sni_tester_adb -f %s%s 2>&1",
		remoteInput, modeArgs)

	validDomainsMap := make(map[string][]string)
	successCount := 0
	failCount := 0
	totalProcessed := 0

	fmt.Println("─────────────────────────────────────────────────────────────")
	fmt.Println("  Progress will be shown below...")
	fmt.Println("─────────────────────────────────────────────────────────────")

	bar := newProgressBar(100)
	bar.Start()

	err = ShellStream(shellCmd, func(line string) {
		line = strings.TrimSpace(line)
		if line == "" {
			return
		}

		if strings.HasPrefix(line, "Starting SNI test") || strings.HasPrefix(line, "Progress:") {
			fmt.Fprintf(os.Stderr, "\r[DEVICE] %s", line)
			return
		}

		if strings.HasPrefix(line, "{") && strings.HasSuffix(line, "}") {
			result, err := ParseResultLine(line)
			if err == nil && result != nil {
				totalProcessed++
				if result.Success {
					successCount++
					country := result.Country
					if country == "" || country == "UNKNOWN" {
						country = "UNKNOWN"
					}
					validDomainsMap[country] = append(validDomainsMap[country], result.Domain)
				} else {
					failCount++
				}
				bar.Add(1)
			}
			return
		}

		if strings.Contains(line, "Completed:") {
			fmt.Fprintf(os.Stderr, "\n[DEVICE] %s\n", line)
			return
		}

		fmt.Println(line)
	})

	bar.Finish()
	fmt.Println()

	if err != nil {
		fmt.Printf("[ERROR] Test execution failed: %v\n", err)
	}

	fmt.Println()
	fmt.Printf("[RESULT] Total: %d, Success: %d, Failed: %d\n", totalProcessed, successCount, failCount)
	fmt.Println()

	fmt.Println("[STEP 7] Importing results to sni directory...")
	baseTargetDir := findTargetDir()
	if baseTargetDir == "" {
		fmt.Println("[ERROR] Could not find sni directory")
		os.Exit(1)
	}

	subDir := ""
	if realityMode {
		subDir = "reality"
	} else if xhttpMode {
		subDir = "xhttp"
	}

	targetDir := baseTargetDir
	if subDir != "" {
		targetDir = filepath.Join(baseTargetDir, subDir)
	}

	os.MkdirAll(targetDir, 0755)

	totalImported := 0
	for country, domains := range validDomainsMap {
		filename := fmt.Sprintf("%s.txt", strings.ToUpper(country))
		targetPath := filepath.Join(targetDir, filename)

		existing := make(map[string]bool)
		if f, err := os.Open(targetPath); err == nil {
			scanner := bufio.NewScanner(f)
			for scanner.Scan() {
				existing[scanner.Text()] = true
			}
			f.Close()
		}

		file, err := os.OpenFile(targetPath, os.O_APPEND|os.O_CREATE|os.O_WRONLY, 0644)
		if err != nil {
			fmt.Printf("[WARN] Could not write %s: %v\n", filename, err)
			continue
		}

		count := 0
		for _, domain := range domains {
			if !existing[domain] {
				file.WriteString(domain + "\n")
				count++
			}
		}
		file.Close()

		if count > 0 {
			fmt.Printf("[OK] %s: %d domains imported\n", country, count)
			totalImported += count
		}
	}

	fmt.Printf("\n[COMPLETE] Total %d domains imported to %s\n", totalImported, targetDir)
	fmt.Println()

	fmt.Println("[STEP 8] Cleaning up device files...")
	cleanupFiles := []string{
		"/data/local/tmp/sni_tester_adb",
		"/data/local/tmp/failed_history.json",
	}
	for _, f := range cleanupFiles {
		RemoveRemoteFile(f)
	}

	inputBase := filepath.Base(inputFile)
	RemoveRemoteFile(fmt.Sprintf("/data/local/tmp/%s", inputBase))
	if _, err := os.Stat(geoDBFile); err == nil {
		RemoveRemoteFile(fmt.Sprintf("/data/local/tmp/%s", filepath.Base(geoDBFile)))
	}

	fmt.Println("[OK] Cleanup completed")
	fmt.Println()
	fmt.Println("╔═══════════════════════════════════════════════════════════╗")
	fmt.Println("║              ADB Mode Completed Successfully              ║")
	fmt.Println("╚═══════════════════════════════════════════════════════════╝")
}

type progressBar struct {
	total   int
	current int
	width   int
}

func newProgressBar(total int) *progressBar {
	return &progressBar{
		total: total,
		width: 50,
	}
}

func (p *progressBar) Start() {
	p.current = 0
}

func (p *progressBar) Add(n int) {
	p.current += n
	if p.current > p.total {
		p.current = p.total
	}
}

func (p *progressBar) Finish() {
	p.current = p.total
	p.render()
	fmt.Println()
}

func (p *progressBar) render() {
	percent := float64(p.current) / float64(p.total)
	filled := int(float64(p.width) * percent)
	empty := p.width - filled

	bar := ""
	for i := 0; i < filled; i++ {
		bar += "█"
	}
	for i := 0; i < empty; i++ {
		bar += "░"
	}

	fmt.Printf("\r[%s] %.1f%% (%d/%d)", bar, percent*100, p.current, p.total)
}
