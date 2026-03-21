package main

import (
	"bufio"
	"context"
	"encoding/json"
	"fmt"
	"net"
	"os"
	"strings"
	"sync"
	"sync/atomic"
	"time"

	"fyne.io/fyne/v2"
	"fyne.io/fyne/v2/app"
	"fyne.io/fyne/v2/dialog"
	"fyne.io/fyne/v2/widget"

	utls "github.com/refraction-networking/utls"
)

const (
	InitialWorkers = 100
	MaxWorkers     = 2000
	MinWorkers     = 10
	JobBuffer      = 5000
)

var DnsPool = []string{
	"1.1.1.1", "1.0.0.1",
	"8.8.8.8", "8.8.4.4",
	"9.9.9.9", "149.112.112.112",
}

var dnsIndex uint32
var dnsCache sync.Map

type ValidationResult struct {
	Domain  string `json:"domain"`
	Success bool   `json:"success"`
	IP      string `json:"ip"`
	Country string `json:"country"`
	Info    string `json:"info"`
}

type TestResult struct {
	Version   string             `json:"version"`
	Mode      string             `json:"mode"`
	Timestamp string             `json:"timestamp"`
	Results   []ValidationResult `json:"results"`
}

func pickClientHelloID() utls.ClientHelloID {
	return utls.HelloChrome_Auto
}

func pickALPNProfile() []string {
	return []string{"h2", "http/1.1"}
}

func checkSNI(domain string, targetIP string, xhttp, reality bool) (bool, string, string) {
	dialer := &net.Dialer{Timeout: 5 * time.Second}
	addr := net.JoinHostPort(targetIP, "443")
	rawConn, err := dialer.DialContext(context.Background(), "tcp", addr)
	if err != nil {
		return false, "", err.Error()
	}
	alpn := pickALPNProfile()
	config := &utls.Config{
		ServerName: domain,
		MinVersion: utls.VersionTLS12,
		MaxVersion: utls.VersionTLS13,
		NextProtos: alpn,
	}
	if reality || xhttp {
		config.MinVersion = utls.VersionTLS13
	}
	helloID := pickClientHelloID()
	uConn := utls.UClient(rawConn, config, helloID)
	defer uConn.Close()
	uConn.SetDeadline(time.Now().Add(10 * time.Second))
	if err := uConn.Handshake(); err != nil {
		return false, "", err.Error()
	}
	state := uConn.ConnectionState()

	if state.Version != utls.VersionTLS13 && (reality || xhttp) {
		return false, "", fmt.Sprintf("Requirement: TLS 1.3 (got %04x)", state.Version)
	}

	info := "Validated"
	return true, targetIP, info
}

func resolveDNS(domain string) (string, error) {
	if cached, ok := dnsCache.Load(domain); ok {
		return cached.(string), nil
	}
	idx := atomic.AddUint32(&dnsIndex, 1) % uint32(len(DnsPool))
	resolver := &net.Resolver{
		PreferGo: true,
		Dial: func(ctx context.Context, network, address string) (net.Conn, error) {
			d := net.Dialer{Timeout: 3 * time.Second}
			return d.DialContext(ctx, "udp4", DnsPool[idx]+":53")
		},
	}
	ips, err := resolver.LookupHost(context.Background(), domain)
	if err != nil || len(ips) == 0 {
		return "", err
	}
	ip := ips[0]
	dnsCache.Store(domain, ip)
	return ip, nil
}

func isBlockedCountry(code string) bool {
	return code == "CN" || code == "HK" || code == "MO" || code == "IR" || code == "RU" || code == "KP"
}

type SNITester struct {
	inputFile string
	mode      string
	isRunning bool
	results   []ValidationResult
	mu        sync.Mutex
	logCb     func(string)
	progressCb func(float64)
}

func NewSNITester() *SNITester {
	return &SNITester{
		results: make([]ValidationResult, 0),
	}
}

func (s *SNITester) SetLogCallback(cb func(string)) {
	s.logCb = cb
}

func (s *SNITester) SetProgressCallback(cb func(float64)) {
	s.progressCb = cb
}

func (s *SNITester) Start() error {
	s.isRunning = true
	s.results = s.results[:0]

	file, err := os.Open(s.inputFile)
	if err != nil {
		return err
	}
	defer file.Close()

	var lines []string
	scanner := bufio.NewScanner(file)
	for scanner.Scan() {
		lines = append(lines, scanner.Text())
	}

	total := len(lines)
	done := 0

	jobs := make(chan string, JobBuffer)
	results := make(chan ValidationResult, MaxWorkers)
	var wg sync.WaitGroup

	for i := 0; i < InitialWorkers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for domain := range jobs {
				ip, err := resolveDNS(domain)
				if err != nil {
					results <- ValidationResult{Domain: domain, Success: false, IP: "", Country: "UNKNOWN", Info: "DNS failed"}
					continue
				}

				success, finalIP, info := checkSNI(domain, ip, s.mode == "XHTTP", s.mode == "Reality")
				if success {
					results <- ValidationResult{Domain: domain, Success: true, IP: finalIP, Country: "US", Info: info}
				} else {
					results <- ValidationResult{Domain: domain, Success: false, IP: ip, Country: "UNKNOWN", Info: info}
				}
			}
		}()
	}

	go func() {
		for domain := range lines {
			jobs <- domain
		}
		close(jobs)
	}()

	go func() {
		for res := range results {
			s.mu.Lock()
			s.results = append(s.results, res)
			s.mu.Unlock()
			done++
			if s.progressCb != nil {
				s.progressCb(float64(done) / float64(total))
			}
			if s.logCb != nil {
				if res.Success {
					s.logCb(fmt.Sprintf("[PASS] %s - %s", res.Domain, res.Info))
				} else {
					s.logCb(fmt.Sprintf("[FAIL] %s - %s", res.Domain, res.Info))
				}
			}
		}
		s.isRunning = false
		if s.logCb != nil {
			s.logCb("测试完成")
		}
	}()

	return nil
}

func (s *SNITester) ExportJSON(path string) error {
	s.mu.Lock()
	defer s.mu.Unlock()

	result := TestResult{
		Version:   "1.0",
		Mode:      s.mode,
		Timestamp: time.Now().Format(time.RFC3339),
		Results:   s.results,
	}

	data, err := json.MarshalIndent(result, "", "  ")
	if err != nil {
		return err
	}

	return os.WriteFile(path, data, 0644)
}

func main() {
	a := app.New()
	w := a.NewWindow("SNI Tester Mobile")
	w.Resize(fyne.NewSize(420, 600))

	tester := NewSNITester()
	inputFile := ""
	logs := []string{}
	logLock := sync.Mutex{}

	updateLog := func(msg string) {
		logLock.Lock()
		logs = append(logs, msg)
		if len(logs) > 100 {
			logs = logs[len(logs)-100:]
		}
		logLock.Unlock()
		logEntry.SetText(strings.Join(logs, "\n"))
	}

	tester.SetLogCallback(updateLog)
	tester.SetProgressCallback(func(p float64) {
		progressBar.SetValue(p)
		progressLabel.SetText(fmt.Sprintf("%.0f%%", p*100))
	})

	fileLabel := widget.NewLabel("未选择文件")
	fileBtn := widget.NewButton("选择文件", func() {
		dialog.ShowFileOpen(func(uri fyne.URIReadCloser, err error) {
			if err != nil || uri == nil {
				return
			}
			inputFile = uri.URI().Path()
			fileLabel.SetText(uri.URI().Name())
			tester.inputFile = inputFile
		}, w)
	})

	modeSelect := widget.NewRadioGroup([]string{"Reality", "XHTTP"}, func(s string) {
		tester.mode = s
	})
	modeSelect.SetSelected("Reality")
	tester.mode = "Reality"

	progressBar := widget.NewProgressBar()
	progressLabel := widget.NewLabel("0%")

	logEntry := widget.NewEntry()
	logEntry.SetPlaceHolder("日志输出...")
	logEntry.MultiLine = true
	logEntry.MinRowsVisible = 8
	logEntry.Disable()

	startBtn := widget.NewButton("开始测试", func() {
		if inputFile == "" {
			dialog.ShowInformation("提示", "请先选择输入文件", w)
			return
		}
		if tester.isRunning {
			return
		}
		startBtn.Disable()
		go func() {
			tester.Start()
			a.SendNotification(fyne.NewNotification("完成", "SNI 测试已完成"))
			startBtn.Enable()
		}()
	})

	exportBtn := widget.NewButton("导出 JSON", func() {
		if len(tester.results) == 0 {
			dialog.ShowInformation("提示", "没有可导出的结果", w)
			return
		}
		dialog.ShowFileSave(func(uri fyne.URIWriteCloser, err error) {
			if err != nil || uri == nil {
				return
			}
			path := uri.URI().Path()
			if !strings.HasSuffix(path, ".json") {
				path += ".json"
			}
			if err := tester.ExportJSON(path); err != nil {
				dialog.ShowError(err, w)
				return
			}
			dialog.ShowInformation("成功", fmt.Sprintf("已导出到: %s", path), w)
		}, w)
	})

	clearBtn := widget.NewButton("清空", func() {
		logLock.Lock()
		logs = nil
		logLock.Unlock()
		logEntry.SetText("")
		progressBar.SetValue(0)
		progressLabel.SetText("0%")
	})

	content := widget.NewVBox(
		widget.NewLabel("SNI Tester Mobile"),
		widget.NewSeparator(),
		widget.NewHBox(widget.NewLabel("文件:"), fileLabel, fileBtn),
		widget.NewLabel("测试模式:"),
		modeSelect,
		widget.NewSeparator(),
		widget.NewHBox(startBtn),
		widget.NewHBox(progressBar, progressLabel),
		widget.NewSeparator(),
		widget.NewLabel("日志:"),
		widget.NewScrollContainer(logEntry),
		widget.NewSeparator(),
		widget.NewHBox(exportBtn, clearBtn),
	)

	w.SetContent(content)
	w.ShowAndRun()
}
