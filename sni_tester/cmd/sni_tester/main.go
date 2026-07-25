package main

import (
	"bufio"
	"context"
	"flag"
	"fmt"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"strings"
	"syscall"

	"github.com/schollz/progressbar/v3"
	"sni_tester/pkg"
	"sni_tester/pkg/isolate"
)

func init() {
	if os.Getuid() == 0 || len(os.Args) < 2 {
		return
	}
	for _, a := range os.Args {
		if a == "--wifi" {
			exe, _ := os.Executable()
			cmd := exec.Command("sudo", append([]string{exe}, os.Args[1:]...)...)
			cmd.Stdin = os.Stdin
			cmd.Stdout = os.Stdout
			cmd.Stderr = os.Stderr
			cmd.Run()
			os.Exit(0)
		}
	}
}

func main() {
	inputFile := flag.String("f", "", "Input TXT/CSV file containing SNIs")
	debugMode := flag.Bool("debug", false, "Enable debug logging")
	dnsAddr := flag.String("dns", "", "DNS server address")
	ttlDays := flag.Int("ttl", 7, "Days to remember failures")
	maxLines := flag.Int("max", 0, "Max lines to read")
	fixedWorkers := flag.Int("w", 0, "Fixed worker count")
	autoShutdown := flag.Bool("shutdown", false, "Shutdown after completion")
	forceRetry := flag.Bool("force", false, "Re-test skipped domains")
	resetAll := flag.Bool("reset", false, "Clear all history")
	proxyString := flag.String("p", "", "Proxy for Geo download")
	wifiMode := flag.Bool("wifi", false, "Wi-Fi namespace isolation")
	wifiIface := flag.String("i", "", "Wi-Fi interface (auto-detect)")
	flag.Parse()

	if *inputFile == "" {
		flag.Usage()
		os.Exit(1)
	}

	if *wifiMode {
		runWiFi(*inputFile, *fixedWorkers, *debugMode, *wifiIface)
		return
	}

	targetDir := findTargetDir()
	if targetDir == "" {
		fmt.Println("Error: Could not find rust/aegis/src/resources/sni directory.")
		os.Exit(1)
	}

	cfg := pkg.DefaultConfig()
	cfg.FixedWorkers = *fixedWorkers
	cfg.ForceRetry = *forceRetry
	cfg.ResetAll = *resetAll
	cfg.TTLDays = *ttlDays
	cfg.MaxLines = *maxLines
	cfg.Debug = *debugMode
	cfg.UseBuiltinDNS = *dnsAddr == ""
	cfg.DNSAddr = *dnsAddr
	cfg.OutputDir = targetDir

	if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, *proxyString); err != nil {
		fmt.Printf("Warning: GeoDB download failed: %v\n", err)
	}

	engine, err := pkg.NewEngine(cfg)
	if err != nil {
		fmt.Printf("Error initializing engine: %v\n", err)
		os.Exit(1)
	}
	defer engine.Close()

	ctx, cancel := context.WithCancel(context.Background())
	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigChan
		cancel()
	}()

	domains := readDomains(*inputFile, cfg.MaxLines)

	bar := progressbar.NewOptions(len(domains),
		progressbar.OptionSetDescription("Testing SNIs"),
		progressbar.OptionShowCount(),
		progressbar.OptionShowIts(),
		progressbar.OptionSetItsString("domains"),
		progressbar.OptionFullWidth(),
	)

	cb := func(event pkg.ProgressEvent) {
		bar.Add(1)
		if event.Type == "validated" && event.Success {
		}
	}

	result, err := engine.Run(ctx, domains, cb)
	if err != nil {
		fmt.Printf("Engine error: %v\n", err)
	}

	fmt.Printf("\nDone. %d succeeded, %d failed, %d skipped\n",
		result.Stats.Success, result.Stats.Failed, result.Stats.Skipped)

	if *autoShutdown {
	}
}

func findTargetDir() string {
	cwd, _ := os.Getwd()
	dir := cwd
	for {
		target := filepath.Join(dir, "rust", "aegis", "src", "resources", "sni")
		if info, err := os.Stat(target); err == nil && info.IsDir() {
			return target
		}
		parent := filepath.Dir(dir)
		if parent == dir {
			break
		}
		dir = parent
	}
	return ""
}

func readDomains(path string, maxLines int) []string {
	file, err := os.Open(path)
	if err != nil {
		fmt.Printf("Error opening file: %v\n", err)
		os.Exit(1)
	}
	defer file.Close()

	var domains []string
	sc := bufio.NewScanner(file)
	for sc.Scan() {
		d := pkg.CleanDomain(sc.Text())
		if d != "" {
			domains = append(domains, d)
			if maxLines > 0 && len(domains) >= maxLines {
				break
			}
		}
	}
	return domains
}

func runWiFi(inputFile string, workers int, debug bool, iface string) {
	targetDir := findTargetDir()
	if targetDir == "" {
		fmt.Fprintln(os.Stderr, "Error: Could not find rust/aegis/src/resources/sni directory")
		os.Exit(1)
	}

	wifi := iface
	if wifi == "" {
		wifi = detectWiFiIface()
		if wifi == "" {
			fmt.Fprintln(os.Stderr, "no Wi-Fi interface found, use -i to specify")
			os.Exit(1)
		}
	}
	fmt.Println("Wi-Fi:", wifi)

	ctrl := isolate.NewController(isolate.ControllerConfig{
		Namespace: "sni-test",
		WiFiIface: wifi,
	})

	fmt.Print("setting up namespace... ")
	if err := ctrl.Setup(); err != nil {
		fmt.Fprintf(os.Stderr, "\nsetup failed: %v\n", err)
		os.Exit(1)
	}
	fmt.Println("ready")

	sigChan := make(chan os.Signal, 1)
	signal.Notify(sigChan, syscall.SIGINT, syscall.SIGTERM)
	go func() {
		<-sigChan
		ctrl.Cleanup()
		os.Exit(0)
	}()
	defer func() {
		ctrl.Cleanup()
		fmt.Println("cleanup complete")
	}()

	domFile := "/tmp/sni-isolate-domains.txt"
	domains := readDomains(inputFile, 0)
	lines := make([]string, len(domains))
	for i, d := range domains {
		lines[i] = d
	}
	os.WriteFile(domFile, []byte(strings.Join(lines, "\n")), 0600)
	defer os.Remove(domFile)

	w := fmt.Sprint(max(workers, 100))

	exe, _ := os.Executable()
	nsArgs := []string{"netns", "exec", "sni-test", exe,
		"-f", domFile,
		"-w", w,
	}
	if debug {
		nsArgs = append(nsArgs, "-debug")
	}
	cmd := exec.Command("ip", nsArgs...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = os.Stdin
	cmd.Run()
}

func detectWiFiIface() string {
	out, err := exec.Command("nmcli", "-t", "-f", "DEVICE,TYPE,STATE", "device", "status").Output()
	if err == nil {
		for _, line := range strings.Split(string(out), "\n") {
			parts := strings.SplitN(line, ":", 3)
			if len(parts) == 3 && parts[1] == "wifi" && parts[2] == "connected" {
				return parts[0]
			}
		}
	}
	entries, _ := os.ReadDir("/sys/class/net")
	for _, e := range entries {
		if _, err := os.Stat("/sys/class/net/" + e.Name() + "/wireless"); err == nil {
			return e.Name()
		}
	}
	return ""
}

func max(a, b int) int {
	if a > b {
		return a
	}
	return b
}
