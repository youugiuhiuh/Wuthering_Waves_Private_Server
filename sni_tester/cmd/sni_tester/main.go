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
	"syscall"

	"github.com/schollz/progressbar/v3"
	"sni_tester/pkg"
)

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
	flag.Parse()

	if *inputFile == "" {
		flag.Usage()
		os.Exit(1)
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
		fmt.Println("Interrupt received, stopping...")
		cancel()
		<-sigChan
		os.Exit(130)
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
		fmt.Println("Testing complete, shutting down...")
		exec.Command("poweroff").Run()
		os.Exit(0)
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
