package main

import (
	"bufio"
	"context"
	"flag"
	"fmt"
	"io"
	"log"
	"os"
	"os/exec"
	"os/signal"
	"path/filepath"
	"syscall"
	"time"

	"github.com/schollz/progressbar/v3"
	"sni_tester/pkg"
)

func main() {
	if len(os.Args) > 1 {
		switch os.Args[1] {
		case "resolve-map":
			handlePreResolve(os.Args[2:])
			return
		case "probe-survival":
			handleProbeSurvival(os.Args[2:])
			return
		case "build-pools":
			handleBuildPools(os.Args[2:])
			return
		}
	}
	inputFile := flag.String("f", "", "Input TXT/CSV file containing SNIs")
	debugMode := flag.Bool("debug", false, "Enable debug logging")
	dnsAddr := flag.String("dns", "", "DNS server address")
	ttlDays := flag.Int("ttl", 7, "Days to remember failures")
	maxLines := flag.Int("max", 0, "Max lines to read")
	fixedWorkers := flag.Int("w", 0, "Fixed worker count")
	autoShutdown := flag.Bool("shutdown", false, "Shutdown after completion")
	forceRetry := flag.Bool("force", false, "Re-test skipped domains")
	resetAll := flag.Bool("reset", false, "Clear all history")
	proxyString := flag.String("p", "", "Outbound proxy URL for all DNS, TLS probes, and GeoDB download (e.g. socks5://127.0.0.1:1080)")
	upgradeDir := flag.String("upgrade-dir", "", "Directly upgrade all .pb files in a directory to dual-pool (PQ/Classic)")
	upgradePb := flag.String("upgrade-pb", "", "Directly upgrade a single .pb file to dual-pool (PQ/Classic)")
	keepDead := flag.Bool("keep-dead", false, "Keep failed domains in classic pool during upgrade")
	outDir := flag.String("out-dir", "", "Output directory for upgraded .pb files (default: overwrite in-place)")
	ifaceName := flag.String("iface", "", "Bind outbound traffic to network interface (e.g. wlp6s0) or IP to isolate from main broadband")
	timeoutSec := flag.Int("timeout", 5, "TLS & DNS timeout in seconds (default 5s for overseas endpoints)")
	flag.Parse()

	if !*debugMode {
		// 非 debug 模式下静音 Go 标准库底层的网络/HTTP2 协议告警（如 received DATA on a HEAD request、UnknownFrame 等），
		// 避免非标野鸡服务器打断刷屏终端进度条；如需排查底层协议细节，可传入 -debug 开启。
		log.SetOutput(io.Discard)
	}

	if *ifaceName != "" {
		if err := pkg.SetOutboundInterface(*ifaceName); err != nil {
			fmt.Printf("Error binding interface %s: %v\n", *ifaceName, err)
			os.Exit(1)
		}
		fmt.Printf("Successfully bound traffic to interface %s (IP: %s)\n", *ifaceName, pkg.GetBoundIP())
	}

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
	defer cancel()

	if *upgradePb != "" {
		workers := *fixedWorkers
		if workers <= 0 {
			workers = 20
		}
		targetOut := *upgradePb
		if *outDir != "" {
			targetOut = filepath.Join(*outDir, filepath.Base(*upgradePb))
		}
		timeout := time.Duration(*timeoutSec) * time.Second
		if timeout <= 0 {
			timeout = 2 * time.Second
		}
		fmt.Printf("Upgrading single PB file: %s -> %s (workers=%d, timeout=%v)\n", *upgradePb, targetOut, workers, timeout)
		stats, err := pkg.UpgradePbFile(ctx, *upgradePb, targetOut, workers, timeout, *keepDead)
		if err != nil {
			fmt.Printf("Error upgrading %s: %v\n", *upgradePb, err)
			os.Exit(1)
		}
		fmt.Printf("Done: Total=%d, PQ=%d, Classic=%d, Dead=%d\n", stats.Total, stats.PQCount, stats.ClassicCount, stats.DeadCount)
		return
	}

	if *upgradeDir != "" {
		workers := *fixedWorkers
		if workers <= 0 {
			workers = 30
		}
		timeout := time.Duration(*timeoutSec) * time.Second
		if timeout <= 0 {
			timeout = 2 * time.Second
		}
		fmt.Printf("Upgrading PB directory: %s (workers=%d, timeout=%v)\n", *upgradeDir, workers, timeout)
		allStats, err := pkg.UpgradePbDir(ctx, *upgradeDir, *outDir, workers, timeout, *keepDead)
		if err != nil {
			fmt.Printf("Error upgrading directory %s: %v\n", *upgradeDir, err)
			os.Exit(1)
		}
		totalPQ := 0
		totalClassic := 0
		totalDead := 0
		totalDomains := 0
		for _, s := range allStats {
			totalDomains += s.Total
			totalPQ += s.PQCount
			totalClassic += s.ClassicCount
			totalDead += s.DeadCount
		}
		fmt.Printf("\n=== Directory Upgrade Complete ===\nFiles: %d | Total Domains: %d | PQ: %d | Classic: %d | Dead: %d\n",
			len(allStats), totalDomains, totalPQ, totalClassic, totalDead)
		return
	}

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

	if *proxyString != "" {
		if err := pkg.SetOutboundProxy(*proxyString); err != nil {
			fmt.Printf("Error setting outbound proxy %q: %v\n", *proxyString, err)
			os.Exit(1)
		}
		fmt.Printf("[Proxy] All outbound TLS probes and DNS queries routed via %s\n", *proxyString)
	}

	if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, *proxyString); err != nil {
		fmt.Printf("Warning: GeoDB download failed: %v\n", err)
	}

	engine, err := pkg.NewEngine(cfg)
	if err != nil {
		fmt.Printf("Error initializing engine: %v\n", err)
		os.Exit(1)
	}
	defer engine.Close()

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

func handleProbeSurvival(args []string) {
	fs := flag.NewFlagSet("probe-survival", flag.ExitOnError)
	inputFile := fs.String("f", "", "Input raw SNI list (TXT/CSV)")
	outputFile := fs.String("o", "survived_candidates.txt", "Output file for survived candidates")
	workers := fs.Int("w", 50, "Worker concurrency")
	timeoutSec := fs.Int("timeout", 3, "Lightweight TCP/TLS probe timeout in seconds")
	proxyString := fs.String("p", "", "Outbound SOCKS5 proxy URL (e.g. socks5://127.0.0.1:1080)")
	maxLines := fs.Int("max", 0, "Max lines to read")
	ifaceName := fs.String("iface", "", "Bind to specific network interface or IP")
	fs.Parse(args)

	if *inputFile == "" {
		fmt.Println("Usage: sni_tester probe-survival -f <input_list> [-o survived_candidates.txt] [-p socks5://...] [-w 50]")
		os.Exit(1)
	}

	if *proxyString != "" {
		if err := pkg.SetOutboundProxy(*proxyString); err != nil {
			fmt.Printf("Error setting outbound proxy %q: %v\n", *proxyString, err)
			os.Exit(1)
		}
		fmt.Printf("[Proxy] Survival probing routed via %s\n", *proxyString)
	} else if *ifaceName != "" {
		if err := pkg.SetOutboundInterface(*ifaceName); err != nil {
			fmt.Printf("Error binding interface %s: %v\n", *ifaceName, err)
			os.Exit(1)
		}
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	fmt.Printf("[Phase 1: Survival Probe] Reading %s -> %s (workers=%d, timeout=%ds)\n", *inputFile, *outputFile, *workers, *timeoutSec)
	opts := pkg.SurvivalOptions{
		InputFile:  *inputFile,
		OutputFile: *outputFile,
		Workers:    *workers,
		Timeout:    time.Duration(*timeoutSec) * time.Second,
		MaxLines:   *maxLines,
	}

	stats, err := pkg.RunProbeSurvival(ctx, opts)
	if err != nil {
		fmt.Printf("Error during survival probe: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("\n[Phase 1 Complete] Total: %d | Survived: %d | Filtered/Dead: %d | Elapsed: %v\nCandidates saved to %s\n",
		stats.Total, stats.Survived, stats.Dead, stats.Duration.Round(time.Millisecond), *outputFile)
}

func handleBuildPools(args []string) {
	fs := flag.NewFlagSet("build-pools", flag.ExitOnError)
	inputFile := fs.String("f", "survived_candidates.txt", "Input survived candidates list from Phase 1")
	outDir := fs.String("out-dir", "rust/aegis/src/resources/sni", "Output directory for country .pb files")
	workers := fs.Int("w", 50, "Worker concurrency for deep TLS inspection")
	timeoutSec := fs.Int("timeout", 5, "Deep inspection timeout in seconds")
	proxyString := fs.String("p", "", "Outbound proxy URL")
	maxLines := fs.Int("max", 0, "Max lines to read")
	geoDB := fs.String("geo", pkg.GeoDBFile, "Path to IPinfo Lite MMDB database")
	fs.Parse(args)

	if *inputFile == "" {
		fmt.Println("Usage: sni_tester build-pools -f <candidates_file> [-out-dir rust/aegis/src/resources/sni] [-w 50]")
		os.Exit(1)
	}

	if *proxyString != "" {
		if err := pkg.SetOutboundProxy(*proxyString); err != nil {
			fmt.Printf("Error setting outbound proxy %q: %v\n", *proxyString, err)
			os.Exit(1)
		}
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	fmt.Printf("[Phase 2: VPS Pool Builder] Deep inspecting %s -> %s (workers=%d, timeout=%ds)\n", *inputFile, *outDir, *workers, *timeoutSec)
	opts := pkg.BuildPoolsOptions{
		CandidatesFile: *inputFile,
		OutputDir:      *outDir,
		Workers:        *workers,
		Timeout:        time.Duration(*timeoutSec) * time.Second,
		GeoDBFile:      *geoDB,
		MaxLines:       *maxLines,
	}

	stats, err := pkg.RunBuildPools(ctx, opts)
	if err != nil {
		fmt.Printf("Error building pools: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("\n[Phase 2 Complete] Total: %d | Post-Quantum (PQ): %d | Classic: %d | Rejected: %d | Elapsed: %v\nDual-pool .pb files written to %s\n",
		stats.Total, stats.PQCount, stats.ClassicCount, stats.Rejected, stats.Duration.Round(time.Millisecond), *outDir)
}

func handlePreResolve(args []string) {
	fs := flag.NewFlagSet("resolve-map", flag.ExitOnError)
	inputFile := fs.String("f", "", "Input raw domain list (TXT/CSV)")
	outputFile := fs.String("o", "resolved_targets.csv", "Output file for pre-resolved domain,ip mapping")
	workers := fs.Int("w", 100, "DNS worker concurrency")
	maxLines := fs.Int("max", 0, "Max lines to read")
	proxyString := fs.String("p", "", "Outbound proxy URL")
	fs.Parse(args)

	if *inputFile == "" {
		fmt.Println("Usage: sni_tester resolve-map -f <raw_domains.csv> [-o resolved_targets.csv] [-w 100]")
		os.Exit(1)
	}

	if *proxyString != "" {
		if err := pkg.SetOutboundProxy(*proxyString); err != nil {
			fmt.Printf("Error setting outbound proxy %q: %v\n", *proxyString, err)
			os.Exit(1)
		}
	}

	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	fmt.Printf("[Phase 1: Overseas Pre-Resolve] Resolving %s -> %s (workers=%d, zero China DNS)\n", *inputFile, *outputFile, *workers)
	opts := pkg.PreResolveOptions{
		InputFile:  *inputFile,
		OutputFile: *outputFile,
		Workers:    *workers,
		MaxLines:   *maxLines,
	}

	stats, err := pkg.RunResolvePreMap(ctx, opts)
	if err != nil {
		fmt.Printf("Error during pre-resolve: %v\n", err)
		os.Exit(1)
	}

	fmt.Printf("\n[Phase 1 Complete] Total: %d | Resolved: %d | Failed/Cloudflare: %d | Elapsed: %v\nTargets saved to %s\n",
		stats.Total, stats.Resolved, stats.Failed, stats.Duration.Round(time.Millisecond), *outputFile)
}
