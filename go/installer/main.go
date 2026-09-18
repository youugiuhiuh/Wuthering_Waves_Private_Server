package main

import (
	"bytes"
	"crypto/rand"
	"crypto/sha256"
	"crypto/subtle"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/go/installer/i18n"

	tea "charm.land/bubbletea/v2"
	"github.com/awnumar/memguard"
	"golang.org/x/sys/unix"
	"golang.org/x/term"
)

const (
	version     = "1.6.1"
	installDir  = "/etc/wwps/aegis"
	binaryName  = "aegis"
	serviceName = "wwps-aegis"
	serviceFile = "/etc/systemd/system/wwps-aegis.service"

	// defaultSimplexPort 是 simplex-chat WebSocket API 的默认端口。
	defaultSimplexPort = "5225"

	// simplexChatVersion 是 simplex-chat 的锁定版本，不能跟随 /releases/latest 浮动。
	// simploxide-client 0.14.0 的版本范围是 MIN_SUPPORTED_VERSION=7.0.0.0 ..
	// MAX_SUPPORTED_VERSION=7.0.0.99，范围外 aegis 连接失败，日志形如
	// "连接 SimpleX WebSocket 失败: Version v... is unsupported by the current client"。
	simplexChatVersion = "v7.0.0"
	simplexRepoOwner   = "simplex-chat"
	simplexRepoName    = "simplex-chat"

	simplexBinaryName  = "simplex-chat"
	simplexServiceName = "wwps-simplex"
	simplexServiceFile = "/etc/systemd/system/wwps-simplex.service"
)

var uninstallServices = []string{"wwps-aegis", "wwps-simplex", "wwps-core", "wwps-box"}

var uninstallPaths = []string{
	"/etc/systemd/system/wwps-aegis.service",
	"/etc/systemd/system/wwps-simplex.service",
	"/etc/systemd/system/wwps-core.service",
	"/etc/systemd/system/wwps-box.service",
	"/etc/init.d/wwps-core",
	"/etc/wwps",
	"/tmp/wwps-core-installer",
	"/tmp/wwps-core-upgrade",
	"/tmp/sing-box-install",
	"/etc/sysctl.d/90-wwps-bbr3-optimize.conf",
	"/etc/systemd/system/apt-daily-upgrade.timer.d/aegis-timezone.conf",
	"/etc/systemd/system/apt-daily.timer.d/aegis-timezone.conf",
}

type releaseRepo struct {
	Owner string
	Name  string
}

var defaultReleaseRepositories = []releaseRepo{
	{Owner: "youugiuhiuh", Name: "Wuthering_Waves_Private_Server"},
}

const releaseAPIBase = "https://api.github.com"

func randomString(n int) string {
	const letters = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
	b := make([]byte, n)
	_, _ = rand.Read(b)
	for i := range b {
		b[i] = letters[int(b[i])%len(letters)]
	}
	return string(b)
}

func parseReleaseRepo(input string) (releaseRepo, bool) {
	trimmed := strings.Trim(strings.TrimSpace(input), "/")
	parts := strings.SplitN(trimmed, "/", 2)
	if len(parts) != 2 {
		return releaseRepo{}, false
	}
	owner := strings.TrimSpace(parts[0])
	name := strings.TrimSpace(parts[1])
	if owner == "" || name == "" {
		return releaseRepo{}, false
	}
	return releaseRepo{Owner: owner, Name: name}, true
}

func configuredReleaseRepositories() []releaseRepo {
	if value := strings.TrimSpace(os.Getenv("AEGIS_RELEASE_REPOSITORIES")); value != "" {
		items := strings.Split(value, ",")
		repos := make([]releaseRepo, 0, len(items))
		for _, item := range items {
			if repo, ok := parseReleaseRepo(item); ok {
				repos = append(repos, repo)
			}
		}
		if len(repos) > 0 {
			return repos
		}
	}

	if value := strings.TrimSpace(os.Getenv("AEGIS_RELEASE_REPOSITORY")); value != "" {
		if repo, ok := parseReleaseRepo(value); ok {
			return []releaseRepo{repo}
		}
	}

	owner := strings.TrimSpace(os.Getenv("AEGIS_RELEASE_OWNER"))
	repo := strings.TrimSpace(os.Getenv("AEGIS_RELEASE_REPO"))
	if owner != "" && repo != "" {
		return []releaseRepo{{Owner: owner, Name: repo}}
	}

	return defaultReleaseRepositories
}

type releaseAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
	URL                string `json:"url"` // 兼容部分 API 使用 url 字段返回下载地址
	Digest             string `json:"digest"`
}

type latestRelease struct {
	TagName string         `json:"tag_name"`
	Body    string         `json:"body"`
	Assets  []releaseAsset `json:"assets"`
}

// ======================== 输出辅助 ===========================

const (
	colorReset   = "\033[0m"
	colorRed     = "\033[31m"
	colorGreen   = "\033[32m"
	colorYellow  = "\033[33m"
	colorSkyBlue = "\033[1;36m"
)

func printColor(color, msg string) {
	fmt.Printf("%s%s%s\n", color, msg, colorReset)
}

func printRed(msg string)     { printColor(colorRed, msg) }
func printGreen(msg string)   { printColor(colorGreen, msg) }
func printYellow(msg string)  { printColor(colorYellow, msg) }
func printSkyBlue(msg string) { printColor(colorSkyBlue, msg) }

func printBanner() {
	printRed("\n==============================================================")
	printGreen(i18n.T("banner.title"))
	printGreen(i18n.T("banner.version", version))
	printGreen(i18n.T("banner.release_mirrors"))
	if repos := configuredReleaseRepositories(); len(repos) > 0 {
		printGreen(i18n.T("banner.release_repo", repos[0].Owner+"/"+repos[0].Name))
	}
	printSkyBlue(i18n.T("banner.manage_hint"))
	printRed("==============================================================")
}

// ======================== 依赖安装 ===========================

func installDependencies() {
	if _, err := exec.LookPath("apt-get"); err == nil {
		printYellow(i18n.T("dep.checking"))
		cmd := exec.Command("apt-get", "install", "-y", "qrencode", "libcap2-bin")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		if err := cmd.Run(); err != nil {
			printYellow(i18n.T("dep.partial_fail"))
		} else {
			printGreen(i18n.T("dep.done"))
		}
	}
}

// ======================== 系统检测 ===========================

func checkRoot() {
	if os.Getuid() != 0 {
		printRed(i18n.T("root.required"))
		os.Exit(1)
	}
}

func checkArch() string {
	arch := runtime.GOARCH
	switch arch {
	case "amd64":
		return "amd64"
	case "arm64":
		return "arm64"
	default:
		printRed(i18n.T("arch.unsupported", arch))
		os.Exit(1)
		return ""
	}
}

// ======================== 命令执行 ===========================

func runCmdSilent(name string, args ...string) error {
	cmd := exec.Command(name, args...)
	return cmd.Run()
}

func runCmdOutputBytes(name string, args ...string) ([]byte, error) {
	cmd := exec.Command(name, args...)
	out, err := cmd.Output()
	if err != nil {
		return nil, err
	}
	return bytes.TrimSpace(out), nil
}

func extractBase32Secret(output []byte) ([]byte, error) {
	// 兼容旧版本 tgbot 可能输出多行日志；仅提取最后一行合法 Base32 密钥。
	// TOTP secret 通常至少 16 位，由 A-Z2-7 组成。
	re := regexp.MustCompile(`^[A-Z2-7]{16,}$`)
	lines := strings.Split(string(output), "\n")
	for i := len(lines) - 1; i >= 0; i-- {
		line := strings.TrimSpace(lines[i])
		if re.MatchString(line) {
			return []byte(line), nil
		}
	}
	return nil, fmt.Errorf("未在输出中找到合法 TOTP Base32 密钥")
}

func zeroBytes(data []byte) {
	for i := range data {
		data[i] = 0
	}
}

func appendJSONEscaped(dst []byte, value []byte) []byte {
	if value == nil {
		return append(dst, "null"...)
	}
	dst = append(dst, '"')
	validUTF8 := utf8.Valid(value)
	for _, b := range value {
		switch b {
		case '\\', '"':
			dst = append(dst, '\\', b)
		case '\b':
			dst = append(dst, '\\', 'b')
		case '\f':
			dst = append(dst, '\\', 'f')
		case '\n':
			dst = append(dst, '\\', 'n')
		case '\r':
			dst = append(dst, '\\', 'r')
		case '\t':
			dst = append(dst, '\\', 't')
		default:
			if b < 0x20 || (!validUTF8 && b > 0x7E) {
				dst = append(dst, '\\', 'u', '0', '0', "0123456789abcdef"[b>>4], "0123456789abcdef"[b&0x0f])
			} else {
				dst = append(dst, b)
			}
		}
	}
	dst = append(dst, '"')
	return dst
}

func readLine() (string, error) {
	buf := make([]byte, 512)
	n, err := os.Stdin.Read(buf)
	if err != nil {
		return "", err
	}
	s := string(bytes.TrimRight(buf[:n], "\n\r"))
	return s, nil
}

type matrixWellKnownResponse struct {
	Homeserver struct {
		BaseURL string `json:"base_url"`
	} `json:"m.homeserver"`
}

func normalizeMatrixMXID(input string) (string, string, error) {
	mxid := input
	if mxid == "" || strings.ContainsAny(mxid, " \t\r\n") {
		return "", "", fmt.Errorf("invalid Matrix MXID")
	}
	if !strings.HasPrefix(mxid, "@") {
		mxid = "@" + mxid
	}
	localpart, server, ok := strings.Cut(strings.TrimPrefix(mxid, "@"), ":")
	if !ok || localpart == "" || server == "" {
		return "", "", fmt.Errorf("invalid Matrix MXID")
	}
	if parsed, err := url.Parse("https://" + server); err != nil || parsed.Host != server {
		return "", "", fmt.Errorf("invalid Matrix MXID")
	}
	return mxid, server, nil
}

func validateMatrixHomeserver(raw string) (string, error) {
	u, err := url.Parse(raw)
	if err != nil || u.Scheme != "https" || u.Host == "" || u.User != nil || u.RawQuery != "" || u.Fragment != "" {
		return "", fmt.Errorf("invalid HTTPS homeserver URL")
	}
	return strings.TrimRight(u.String(), "/"), nil
}

func discoverMatrixHomeserver(mxid string, client *http.Client) (string, string, error) {
	normalized, server, err := normalizeMatrixMXID(mxid)
	if err != nil {
		return "", "", err
	}
	if client == nil {
		client = &http.Client{Timeout: 5 * time.Second}
	}
	resp, err := client.Get("https://" + server + "/.well-known/matrix/client")
	if err != nil {
		return "", "", err
	}
	defer resp.Body.Close()
	if resp.StatusCode < http.StatusOK || resp.StatusCode >= http.StatusMultipleChoices {
		return "", "", fmt.Errorf("homeserver discovery returned HTTP %d", resp.StatusCode)
	}
	var body matrixWellKnownResponse
	if err := json.NewDecoder(resp.Body).Decode(&body); err != nil {
		return "", "", err
	}
	homeserver, err := validateMatrixHomeserver(body.Homeserver.BaseURL)
	if err != nil {
		return "", "", err
	}
	return normalized, homeserver, nil
}

type platformSelector struct {
	cursor                               int
	telegram, matrix, simplex, confirmed bool
}

func newPlatformSelector() platformSelector { return platformSelector{} }

func (m platformSelector) Init() tea.Cmd { return nil }

func (m platformSelector) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	key, ok := msg.(tea.KeyPressMsg)
	if !ok {
		return m, nil
	}

	switch key.String() {
	case "ctrl+c":
		return m, tea.Quit
	case "up":
		m.cursor = (m.cursor + 2) % 3
	case "down":
		m.cursor = (m.cursor + 1) % 3
	case "space":
		switch m.cursor {
		case 0:
			// Telegram 可作为主平台与 SimpleX 组合，因此不再清空 SimpleX。
			m.telegram = !m.telegram
		case 1:
			m.matrix = !m.matrix
			if m.matrix {
				m.simplex = false
			}
		case 2:
			m.simplex = !m.simplex
			if m.simplex {
				m.matrix = false
			}
		}
	case "enter":
		if _, _, _, valid := m.platformSelection(); valid {
			m.confirmed = true
			return m, tea.Quit
		}
	}
	return m, nil
}

func (m platformSelector) platformSelection() (bool, bool, bool, bool) {
	// SimpleX 只允许与 Telegram 组合（TG 主 + SimpleX 敏感内容落点）；Matrix 与 SimpleX
	// 是两个互斥的独立平台，三者同选同样非法。
	valid := (m.telegram || m.matrix || m.simplex) && !(m.simplex && m.matrix)
	return m.telegram, m.matrix, m.simplex, valid
}

func (m platformSelector) View() tea.View {
	labels := []string{
		i18n.T("firsttime.platform_selector_telegram"),
		i18n.T("firsttime.platform_selector_matrix"),
		i18n.T("firsttime.platform_selector_simplex"),
	}
	selected := make([]string, 0, 2)
	choices := []bool{m.telegram, m.matrix, m.simplex}
	for index, label := range labels {
		prefix := "  "
		if index == m.cursor {
			prefix = "> "
		}
		marker := " "
		if choices[index] {
			marker = "x"
			selected = append(selected, label)
		}
		labels[index] = fmt.Sprintf("%s[%s] %s", prefix, marker, label)
	}

	summary := strings.Join(selected, " + ")
	if summary == "" {
		summary = i18n.T("firsttime.platform_selector_none")
	}
	return tea.NewView(fmt.Sprintf("%s\n%s\n\n%s\n\n%s", i18n.T("firsttime.platform_selector_title"), i18n.T("firsttime.platform_selector_help"), strings.Join(labels, "\n"), i18n.T("firsttime.platform_selector_selected", summary)))
}

func parsePlatformChoice(choice string) (bool, bool, bool, error) {
	switch strings.ToLower(strings.ReplaceAll(strings.TrimSpace(choice), " ", "")) {
	case "telegram":
		return true, false, false, nil
	case "matrix":
		return false, true, false, nil
	case "simplex":
		return false, false, true, nil
	case "telegram+matrix":
		return true, true, false, nil
	case "telegram+simplex":
		return true, false, true, nil
	default:
		return false, false, false, fmt.Errorf("invalid platform")
	}
}

func usesInteractivePlatformSelector(stdinIsTerminal, stdoutIsTerminal bool) bool {
	return stdinIsTerminal && stdoutIsTerminal
}

func selectDeploymentPlatforms() (bool, bool, bool, error) {
	if !usesInteractivePlatformSelector(term.IsTerminal(int(os.Stdin.Fd())), term.IsTerminal(int(os.Stdout.Fd()))) {
		fmt.Print(i18n.T("firsttime.platform_text_prompt"))
		choice, err := readLine()
		if err != nil {
			return false, false, false, err
		}
		return parsePlatformChoice(choice)
	}

	model, err := tea.NewProgram(newPlatformSelector()).Run()
	if err != nil {
		return false, false, false, err
	}
	selector := model.(platformSelector)
	tg, matrix, simplex, valid := selector.platformSelection()
	if !selector.confirmed || !valid {
		return false, false, false, fmt.Errorf("platform selection cancelled")
	}
	return tg, matrix, simplex, nil
}

func usesManualHomeserverFallback(isTerminal bool, discoveryErr error) bool {
	return isTerminal && discoveryErr != nil
}

func selectMatrixHomeserver(mxid string) (normalizedMXID, homeserver string, err error) {
	normalizedMXID, _, err = normalizeMatrixMXID(mxid)
	if err != nil {
		return "", "", err
	}

	printYellow(i18n.T("firsttime.matrix_discovering"))
	normalizedMXID, homeserver, err = discoverMatrixHomeserver(normalizedMXID, &http.Client{Timeout: 5 * time.Second})
	if err == nil {
		return normalizedMXID, homeserver, nil
	}
	if !usesManualHomeserverFallback(term.IsTerminal(int(os.Stdin.Fd())) && term.IsTerminal(int(os.Stdout.Fd())), err) {
		return "", "", err
	}

	printYellow(i18n.T("firsttime.matrix_discovery_failed", err.Error()))
	fmt.Print(i18n.T("firsttime.matrix_manual_hs_prompt"))
	homeserver, err = readLine()
	if err != nil {
		return "", "", err
	}
	homeserver, err = validateMatrixHomeserver(homeserver)
	if err != nil {
		return "", "", err
	}
	return normalizedMXID, homeserver, nil
}

func readSecureInputStr(prompt string) string {
	fmt.Print(prompt)
	buf := make([]byte, 512)
	n, err := os.Stdin.Read(buf)
	if err != nil {
		printRed(i18n.T("input.read_failed", err.Error()))
		os.Exit(1)
	}
	s := strings.TrimRight(string(buf[:n]), "\n\r")
	return s
}

func buildSetupPayload(token, adminID, totpSecret []byte, matrixHS, matrixUser, matrixRoom string, matrixPass, matrixStorePassphrase []byte, matrixRecoveryKey, simplexPort, simplexAdminID string) []byte {
	buf := make([]byte, 0, 256)
	buf = append(buf, '{')
	first := true

	if len(token) > 0 {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"token":`)...)
		buf = appendJSONEscaped(buf, token)
	}
	if len(adminID) > 0 {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"admin_id":`)...)
		buf = appendJSONEscaped(buf, adminID)
	}
	if len(totpSecret) > 0 {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"totp_secret":`)...)
		buf = appendJSONEscaped(buf, totpSecret)
	}
	if matrixHS != "" {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"matrix_homeserver":`)...)
		buf = appendJSONEscaped(buf, []byte(matrixHS))
	}
	if matrixUser != "" {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"matrix_username":`)...)
		buf = appendJSONEscaped(buf, []byte(matrixUser))
	}
	if len(matrixPass) > 0 {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"matrix_password":`)...)
		buf = appendJSONEscaped(buf, matrixPass)
	}
	if matrixRoom != "" {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"matrix_room_id":`)...)
		buf = appendJSONEscaped(buf, []byte(matrixRoom))
	}
	if len(matrixStorePassphrase) > 0 {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"matrix_store_passphrase":`)...)
		buf = appendJSONEscaped(buf, matrixStorePassphrase)
	}
	if matrixRecoveryKey != "" {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"matrix_recovery_key":`)...)
		buf = appendJSONEscaped(buf, []byte(matrixRecoveryKey))
	}
	if simplexPort != "" {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"simplex_port":`)...)
		buf = appendJSONEscaped(buf, []byte(simplexPort))
	}
	if simplexAdminID != "" {
		if !first {
			buf = append(buf, ',')
		}
		first = false
		buf = append(buf, []byte(`"simplex_admin_id":`)...)
		buf = appendJSONEscaped(buf, []byte(simplexAdminID))
	}

	buf = append(buf, '}')
	return buf
}

func buildOtpAuthURL(secret []byte) []byte {
	payload := make([]byte, 0, len(secret)+96)
	payload = append(payload, []byte("otpauth://totp/wwps:admin?secret=")...)
	payload = append(payload, secret...)
	payload = append(payload, []byte("&issuer=wwps&algorithm=SHA512&digits=6&period=30")...)
	return payload
}

func writeLine(prefix string, data []byte) {
	fmt.Print(prefix)
	_, _ = os.Stdout.Write(data)
	_, _ = os.Stdout.Write([]byte("\n"))
}

func disableCoreDumps() {
	if runtime.GOOS != "linux" {
		return
	}

	limit := &unix.Rlimit{Cur: 0, Max: 0}
	if err := unix.Setrlimit(unix.RLIMIT_CORE, limit); err != nil {
		printYellow(i18n.T("warning.core_dump", err.Error()))
	}
	if err := unix.Prctl(unix.PR_SET_DUMPABLE, 0, 0, 0, 0); err != nil {
		printYellow(i18n.T("warning.dumpable", err.Error()))
	}
}

// ======================== 下载和校验 =========================

func newHTTPClient(timeout time.Duration) *http.Client {
	return &http.Client{Timeout: timeout}
}

func downloadFile(client *http.Client, url, dest string) error {
	printYellow(i18n.T("download.start", url))

	resp, err := client.Get(url)
	if err != nil {
		return fmt.Errorf("HTTP 请求失败: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return fmt.Errorf("HTTP 状态码: %d", resp.StatusCode)
	}

	out, err := os.Create(dest)
	if err != nil {
		return fmt.Errorf("创建文件失败: %w", err)
	}
	defer out.Close()

	written, err := io.Copy(out, resp.Body)
	if err != nil {
		return fmt.Errorf("写入失败: %w", err)
	}

	printGreen(i18n.T("download.complete", written))
	return nil
}

func downloadText(client *http.Client, url string) (string, error) {
	resp, err := client.Get(url)
	if err != nil {
		return "", fmt.Errorf("HTTP 请求失败: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return "", fmt.Errorf("HTTP 状态码: %d", resp.StatusCode)
	}

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return "", fmt.Errorf("读取响应失败: %w", err)
	}

	return string(body), nil
}

func sha256File(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()

	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

func getLatestReleaseInfo() (*latestRelease, error) {
	client := newHTTPClient(30 * time.Second)
	var errors []string
	for _, repo := range configuredReleaseRepositories() {
		apiPath := fmt.Sprintf("/repos/%s/%s/releases/latest", repo.Owner, repo.Name)
		apiURL := releaseAPIBase + apiPath
		resp, err := client.Get(apiURL)
		if err != nil {
			errors = append(errors, fmt.Sprintf("%s/%s via %s: %v", repo.Owner, repo.Name, releaseAPIBase, err))
			continue
		}
		if resp.StatusCode != http.StatusOK {
			resp.Body.Close()
			errors = append(errors, fmt.Sprintf("%s/%s via %s 返回状态码: %d", repo.Owner, repo.Name, releaseAPIBase, resp.StatusCode))
			continue
		}
		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			errors = append(errors, fmt.Sprintf("%s/%s via %s 读取失败: %v", repo.Owner, repo.Name, releaseAPIBase, err))
			continue
		}
		var release latestRelease
		if err := json.Unmarshal(body, &release); err != nil {
			errors = append(errors, fmt.Sprintf("%s/%s via %s 解析 JSON 失败: %v", repo.Owner, repo.Name, releaseAPIBase, err))
			continue
		}
		if release.TagName == "" {
			errors = append(errors, fmt.Sprintf("%s/%s via %s release 缺少 tag_name", repo.Owner, repo.Name, releaseAPIBase))
			continue
		}
		return &release, nil
	}
	if len(errors) > 0 {
		return nil, fmt.Errorf("所有 Release 源均失败: %s", strings.Join(errors, " | "))
	}
	return nil, fmt.Errorf("未配置 Release 源")
}

func findAsset(release *latestRelease, name string) *releaseAsset {
	for i := range release.Assets {
		if release.Assets[i].Name == name {
			return &release.Assets[i]
		}
	}
	return nil
}

// assetDownloadURL 返回该 asset 的下载地址，兼容不同 API 的下载地址字段。
func assetDownloadURL(a *releaseAsset, fallbackTemplate string) string {
	if a.BrowserDownloadURL != "" {
		return a.BrowserDownloadURL
	}
	if a.URL != "" {
		return a.URL
	}
	return fallbackTemplate
}

func extractSHA256FromText(content string) string {
	re := regexp.MustCompile(`(?i)\b([0-9a-f]{64})\b`)
	match := re.FindStringSubmatch(content)
	if len(match) == 2 {
		return strings.ToLower(match[1])
	}
	return ""
}

func findExpectedSHA256(release *latestRelease, assetName string) (string, error) {
	client := newHTTPClient(30 * time.Second)

	if checksumAsset := findAsset(release, assetName+".sha256"); checksumAsset != nil {
		checksumURL := assetDownloadURL(checksumAsset, "")
		if checksumURL == "" {
			return "", fmt.Errorf("校验文件 asset 无下载地址")
		}
		content, err := downloadText(client, checksumURL)
		if err != nil {
			return "", fmt.Errorf("下载校验文件失败: %w", err)
		}
		hash := extractSHA256FromText(content)
		if hash != "" {
			return hash, nil
		}
		return "", fmt.Errorf("校验文件中未找到 SHA-256")
	}

	if binaryAsset := findAsset(release, assetName); binaryAsset != nil {
		if digest, ok := strings.CutPrefix(strings.ToLower(binaryAsset.Digest), "sha256:"); ok && digest != "" {
			return digest, nil
		}
	}

	hash := extractSHA256FromText(release.Body)
	if hash != "" {
		return hash, nil
	}

	return "", fmt.Errorf("未找到 %s 的可信 SHA-256", assetName)
}

func verifySHA256(path, expected string) error {
	actual, err := sha256File(path)
	if err != nil {
		return err
	}

	printYellow(i18n.T("sha256.label", actual))
	if subtle.ConstantTimeCompare([]byte(strings.ToLower(actual)), []byte(strings.ToLower(expected))) != 1 {
		return fmt.Errorf("SHA-256 不匹配: expected %s, got %s", expected, actual)
	}
	return nil
}

// ======================== SimpleX 部署 ========================

// validateSimplexPort 校验端口号。
//
// 安全边界：port 来自 key=val / stdin / 交互式输入，最终被字符串拼接进一个 root
// 拥有的 systemd 单元。裸拼接意味着换行可以注入任意指令（例如额外的 ExecStartPre），
// 而 "5225 --host 0.0.0.0" 会把本该只监听 localhost 的无鉴权 API 暴露出去。因此
// 这里只接受 1-65535 的纯数字串，其它一律拒绝。
func validateSimplexPort(port string) error {
	if port == "" {
		return fmt.Errorf("端口不能为空")
	}
	if len(port) > 5 {
		return fmt.Errorf("端口 %q 不是 1-65535 的整数", port)
	}
	for _, r := range port {
		if r < '0' || r > '9' {
			return fmt.Errorf("端口 %q 只能包含数字", port)
		}
	}
	n, err := strconv.Atoi(port)
	if err != nil {
		return fmt.Errorf("端口 %q 不是合法整数: %w", port, err)
	}
	if n < 1 || n > 65535 {
		return fmt.Errorf("端口 %d 超出 1-65535", n)
	}
	return nil
}

// replaceFileAtomically 用 src 的内容原子替换 dest：先写 dest+".new"，fsync 后再
// rename 覆盖。任何一步失败都清理临时文件并让 dest 保持原样。
//
// 为什么不能用就地 O_TRUNC：simplex-chat 由本安装器自己写的 systemd 单元以
// Restart=always 常驻，dest 通常正是那个正在被执行的 inode，Linux 会拒绝写它
// （ETXTBSY），于是第二次及以后的重装必然失败。rename 换的是目录项，与正在被
// 执行的 inode 无关，因此不受影响。
func replaceFileAtomically(dest string, src io.Reader, mode os.FileMode) error {
	tmp := dest + ".new"
	out, err := os.OpenFile(tmp, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, mode)
	if err != nil {
		return fmt.Errorf("创建临时文件失败: %w", err)
	}
	if _, err := io.Copy(out, src); err != nil {
		out.Close()
		_ = os.Remove(tmp)
		return fmt.Errorf("写入临时文件失败: %w", err)
	}
	if err := out.Sync(); err != nil {
		out.Close()
		_ = os.Remove(tmp)
		return fmt.Errorf("同步临时文件失败: %w", err)
	}
	if err := out.Close(); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("关闭临时文件失败: %w", err)
	}
	if err := os.Rename(tmp, dest); err != nil {
		_ = os.Remove(tmp)
		return fmt.Errorf("替换 %s 失败: %w", dest, err)
	}
	return nil
}

// installSimplexBinary 把已下载（并已校验 SHA-256）的 simplex-chat 放进 destDir，
// 返回安装路径。用原子替换而不是就地覆盖，理由见 replaceFileAtomically。
func installSimplexBinary(downloaded, destDir string) (string, error) {
	src, err := os.Open(downloaded)
	if err != nil {
		return "", fmt.Errorf("读取 simplex-chat 失败: %w", err)
	}
	defer src.Close()

	dest := filepath.Join(destDir, simplexBinaryName)
	if err := replaceFileAtomically(dest, src, 0o755); err != nil {
		return "", err
	}
	return dest, nil
}

// simplexChatAssetName 返回 simplex-chat release 中对应架构的预编译产物名。
// 未知架构返回空串，调用方必须当作硬错误处理——静默换一个架构的二进制会装出
// 一个根本跑不起来的服务。
func simplexChatAssetName(arch string) string {
	switch arch {
	case "amd64":
		return "simplex-chat-ubuntu-24_04-x86_64"
	case "arm64":
		return "simplex-chat-ubuntu-24_04-aarch64"
	}
	return ""
}

// simplexDataDir 是 simplex-chat 的数据库与文件目录，与 matrix_store 一样放在 installDir 下。
func simplexDataDir() string {
	return filepath.Join(installDir, "simplex_store")
}

// simplexSystemdUnitContent 生成 simplex-chat 的 systemd 单元。
//
// 安全约束：SimpleX 的 WebSocket API 不做任何鉴权，simplex-chat 默认只绑定到
// localhost（实测 `-p` 监听 127.0.0.1，且 CLI 没有任何改监听地址的开关）。
// 单元里只能传端口，绝不能出现任何改成对外监听的参数，否则等于把一个无鉴权的
// API 暴露到公网。
//
// 无人值守约束（实测验证）：
//   - 全新机器上直接跑 `simplex-chat -p PORT` 会因为「没有 user profile」而停在
//     交互式提问，stdin 是 /dev/null 时直接退出；配合 Restart=always 就是无限
//     崩溃重启。--create-bot-display-name 只在首次启动创建 profile，后续启动是
//     空操作，因此可以常驻在 ExecStart 里。
//   - -y/--yes-migrate 让数据库迁移不会停下来等确认。
//   - -d/--files-folder 显式钉住数据位置，不依赖 root 的 HOME 或工作目录。
//   - --create-bot-allow-files 让 bot 能收发文件，与客户端侧 can_send_file 能力对齐。
func simplexSystemdUnitContent(port string) string {
	dataDir := simplexDataDir()
	return `[Unit]
Description=WWPS SimpleX Chat CLI (WebSocket bot API)
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=` + dataDir + `
ExecStart=` + filepath.Join(installDir, simplexBinaryName) + ` -d ` + filepath.Join(dataDir, "simplex_v1") + ` --create-bot-display-name Aegis --create-bot-allow-files -y --files-folder ` + filepath.Join(dataDir, "files") + ` -p ` + port + `
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
`
}

// simplexUnitForWrite 生成可落盘的单元内容。端口非法时返回错误，调用方不得写文件——
// 端口会被字符串拼接进一个 root 拥有的单元，这里是唯一的落盘闸门。
func simplexUnitForWrite(port string) ([]byte, error) {
	if err := validateSimplexPort(port); err != nil {
		return nil, err
	}
	return []byte(simplexSystemdUnitContent(port)), nil
}

func writeSimplexSystemdService(port string) {
	content, err := simplexUnitForWrite(port)
	if err != nil {
		printRed(i18n.T("simplex.port_invalid", err.Error()))
		return
	}
	if err := os.WriteFile(simplexServiceFile, content, 0o644); err != nil {
		printRed(i18n.T("simplex.unit_write_failed", err.Error()))
	}
}

// simplexPortFromUnit 从已存在的单元文件里取回端口。重装（recovery 路径）时拿不到
// 用户当初填的端口，若不回读就会把自定义端口改回默认值，直接打断一个正在工作的部署。
func simplexPortFromUnit(content []byte) string {
	re := regexp.MustCompile(`(?m)^ExecStart=.*\s-p\s+(\d{1,5})\s*$`)
	if m := re.FindSubmatch(content); len(m) == 2 {
		return string(m[1])
	}
	return ""
}

// simplexReleaseInfo 拉取锁定版本的 simplex-chat release（tags/<version>，不是 latest）。
func simplexReleaseInfo() (*latestRelease, error) {
	client := newHTTPClient(30 * time.Second)
	apiURL := fmt.Sprintf("%s/repos/%s/%s/releases/tags/%s", releaseAPIBase, simplexRepoOwner, simplexRepoName, simplexChatVersion)
	resp, err := client.Get(apiURL)
	if err != nil {
		return nil, fmt.Errorf("请求 simplex-chat release 失败: %w", err)
	}
	defer resp.Body.Close()

	if resp.StatusCode != http.StatusOK {
		return nil, fmt.Errorf("simplex-chat release 返回状态码: %d", resp.StatusCode)
	}
	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, fmt.Errorf("读取 simplex-chat release 失败: %w", err)
	}
	var release latestRelease
	if err := json.Unmarshal(body, &release); err != nil {
		return nil, fmt.Errorf("解析 simplex-chat release 失败: %w", err)
	}
	if release.TagName == "" {
		return nil, fmt.Errorf("simplex-chat release %s 缺少 tag_name", simplexChatVersion)
	}
	return &release, nil
}

// installSimplexChat 下载并安装锁定版本的 simplex-chat 到 installDir，返回安装路径。
//
// 完整性校验复用既有 findExpectedSHA256：v7.0.0 并没有发布 <asset>.sha256 文件，
// 但 release API 为每个资产提供了 sha256 digest，该 helper 会命中 digest 分支。
// 上游另有 _sha256sums / _sha256sums.asc（GPG 签名），本项目未内置 SimpleX 的
// GPG 公钥，因此不校验该签名。
func installSimplexChat() (string, error) {
	assetName := simplexChatAssetName(runtime.GOARCH)
	if assetName == "" {
		return "", fmt.Errorf("架构 %s 没有对应的 simplex-chat 预编译产物", runtime.GOARCH)
	}

	release, err := simplexReleaseInfo()
	if err != nil {
		return "", err
	}
	asset := findAsset(release, assetName)
	if asset == nil {
		return "", fmt.Errorf("simplex-chat %s 缺少资产 %s", simplexChatVersion, assetName)
	}

	fallback := fmt.Sprintf("https://github.com/%s/%s/releases/download/%s/%s", simplexRepoOwner, simplexRepoName, simplexChatVersion, assetName)
	downloadURL := assetDownloadURL(asset, fallback)
	if downloadURL == "" {
		return "", fmt.Errorf("资产 %s 没有下载地址", assetName)
	}

	tmpDir, err := os.MkdirTemp("", "wwps-simplex-*")
	if err != nil {
		return "", fmt.Errorf("创建临时目录失败: %w", err)
	}
	defer os.RemoveAll(tmpDir)

	printYellow(i18n.T("simplex.download_start", simplexChatVersion, assetName))
	downloaded := filepath.Join(tmpDir, assetName)
	if err := downloadFile(newHTTPClient(10*time.Minute), downloadURL, downloaded); err != nil {
		return "", fmt.Errorf("下载 simplex-chat 失败: %w", err)
	}
	if info, err := os.Stat(downloaded); err != nil || info.Size() == 0 {
		return "", fmt.Errorf("下载的 simplex-chat 文件无效")
	}

	expected, err := findExpectedSHA256(release, assetName)
	if err != nil {
		return "", fmt.Errorf("获取 simplex-chat 可信 SHA-256 失败: %w", err)
	}
	if err := verifySHA256(downloaded, expected); err != nil {
		return "", fmt.Errorf("simplex-chat 校验失败: %w", err)
	}

	if err := os.MkdirAll(installDir, 0o755); err != nil {
		return "", fmt.Errorf("创建安装目录失败: %w", err)
	}

	dest, err := installSimplexBinary(downloaded, installDir)
	if err != nil {
		return "", err
	}

	printGreen(i18n.T("simplex.installed", dest))
	return dest, nil
}

// deploySimplexService 安装 simplex-chat 并写好它的 systemd 单元；非 simplex / tg-simplex 平台是空操作。
// 端口优先级：调用方显式给的 > 已存在单元里回读的 > 默认端口。
func deploySimplexService(platform, port string) {
	if platform != "simplex" && platform != "tg-simplex" {
		return
	}
	// 端口先解析并校验，再下载：port 来自 key=val / stdin / 交互输入，最终会拼进
	// root 拥有的 systemd 单元，任何非纯数字值都必须在这里被挡住。先校验也避免了
	// 下载完上百 MB 才因为端口非法而失败。
	if port == "" {
		if existing, err := os.ReadFile(simplexServiceFile); err == nil {
			port = simplexPortFromUnit(existing)
		}
	}
	if port == "" {
		port = defaultSimplexPort
	}
	if err := validateSimplexPort(port); err != nil {
		printRed(i18n.T("simplex.port_invalid", err.Error()))
		os.Exit(1)
	}
	if _, err := installSimplexChat(); err != nil {
		printRed(i18n.T("simplex.install_failed", err.Error()))
		os.Exit(1)
	}
	if err := os.MkdirAll(simplexDataDir(), 0o700); err != nil {
		printRed(i18n.T("simplex.install_failed", err.Error()))
		os.Exit(1)
	}
	writeSimplexSystemdService(port)
	_ = runCmdSilent("systemctl", "daemon-reload")
	if err := runCmdSilent("systemctl", "enable", simplexServiceName); err != nil {
		printRed(i18n.T("simplex.service_failed", err.Error()))
		os.Exit(1)
	}
	// 必须 restart 而不是 enable --now：--now 对一个已经在运行的单元是空操作，
	// 升级时改了端口就会让 simplex-chat 继续监听旧端口，而随后重启的 aegis 用的是
	// 新端口，bot 永远连不上。
	if err := runCmdSilent("systemctl", "restart", simplexServiceName); err != nil {
		printRed(i18n.T("simplex.service_failed", err.Error()))
		os.Exit(1)
	}
	printGreen(i18n.T("simplex.service_ok"))
}

// ======================== 安装 ==============================

func downloadAndDeployAegis() string {
	installDependencies()

	release, err := getLatestReleaseInfo()
	if err != nil {
		printRed(i18n.T("release.fetch_failed", err.Error()))
		return ""
	}

	ver := release.TagName
	printYellow(i18n.T("release.target_version", ver))

	tmpDir, err := os.MkdirTemp("", "wwps-installer-*")
	if err != nil {
		printRed(i18n.T("release.tmpdir_failed", err.Error()))
		return ""
	}
	defer os.RemoveAll(tmpDir)

	binaryPath := filepath.Join(tmpDir, binaryName)
	repositories := configuredReleaseRepositories()
	primaryRepo := defaultReleaseRepositories[0]
	if len(repositories) > 0 {
		primaryRepo = repositories[0]
	}
	fallbackDownload := fmt.Sprintf("https://github.com/%s/%s/releases/download/%s/%s", primaryRepo.Owner, primaryRepo.Name, ver, binaryName)
	asset := findAsset(release, binaryName)
	downloadURL := fallbackDownload
	if asset != nil {
		if u := assetDownloadURL(asset, fallbackDownload); u != "" {
			downloadURL = u
		}
	}

	if err := downloadFile(newHTTPClient(10*time.Minute), downloadURL, binaryPath); err != nil {
		printRed(i18n.T("download.failed", err.Error()))
		return ""
	}

	info, err := os.Stat(binaryPath)
	if err != nil || info.Size() == 0 {
		printRed(i18n.T("download.invalid_file"))
		return ""
	}

	// --- Minisign verification ---
	printYellow(i18n.T("minisign.download_start"))
	assetMinisig := findMinisigAsset(release, binaryName)
	var minisigPassed bool
	if assetMinisig != nil {
		sigURL := assetDownloadURL(assetMinisig, fallbackDownload+".minisig")
		if sigURL != "" {
			sigPath := filepath.Join(tmpDir, binaryName+".minisig")
			if err := downloadFile(newHTTPClient(30*time.Second), sigURL, sigPath); err != nil {
				printRed(i18n.T("minisign.verify_failed", err.Error()))
				return ""
			}
			printYellow(i18n.T("minisign.verify_start"))
			info, err := verifyMinisign(binaryPath, sigPath, minisignActiveKeys, minisignHistoricalKeys)
			if err != nil {
				printRed(i18n.T("minisign.verify_failed", err.Error()))
				return ""
			}
			expectedVersion := ver
			gotVersion, gotAsset, err := parseTrustedComment(info.TrustedComment)
			if err != nil {
				printRed(i18n.T("minisign.verify_failed", err.Error()))
				return ""
			}
			if err := matchTrustedComment(gotVersion, gotAsset, expectedVersion, binaryName); err != nil {
				printRed(err.Error())
				return ""
			}
			printGreen(i18n.T("minisign.verify_ok"))
			printYellow(i18n.T("minisign.trusted_comment", info.TrustedComment))
			minisigPassed = true
		}
	}
	// 硬校验：签名缺失即拒绝，不得回退到「仅 SHA256」。
	// 否则攻击者只需删除 .minisig 资产即可完全绕过签名验证。
	if err := requireMinisign(minisigPassed); err != nil {
		printRed(err.Error())
		return ""
	}

	// --- SHA256 verification ---
	expectedHash, err := findExpectedSHA256(release, binaryName)
	if err != nil {
		printRed(i18n.T("sha256.fetch_failed", err.Error()))
		return ""
	}
	if err := verifySHA256(binaryPath, expectedHash); err != nil {
		printRed(i18n.T("sha256.verify_failed", err.Error()))
		return ""
	}

	if err := os.MkdirAll(installDir, 0o755); err != nil {
		printRed(i18n.T("install.mkdir_failed", err.Error()))
		return ""
	}

	_ = runCmdSilent("systemctl", "stop", serviceName)

	destPath := filepath.Join(installDir, binaryName)
	src, err := os.Open(binaryPath)
	if err != nil {
		printRed(i18n.T("install.read_bin_failed", err.Error()))
		return ""
	}
	defer src.Close()

	dst, err := os.OpenFile(destPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o755)
	if err != nil {
		printRed(i18n.T("install.write_bin_failed", err.Error()))
		return ""
	}
	defer dst.Close()

	if _, err := io.Copy(dst, src); err != nil {
		printRed(i18n.T("install.copy_failed", err.Error()))
		return ""
	}
	dst.Close()
	src.Close()

	printGreen(i18n.T("install.bin_deployed"))

	if err := runCmdSilent("setcap", "cap_ipc_lock+eip", destPath); err != nil {
		printYellow(i18n.T("install.cap_ipc_failed"))
	} else {
		printGreen(i18n.T("install.mem_protect_ok"))
	}

	return destPath
}

func installAegis() {
	printSkyBlue(i18n.T("install.start"))

	configPath := filepath.Join(installDir, "config.enc")
	var platform string
	var simplexPort string
	configExists := false
	if _, err := os.Stat(configPath); err == nil {
		service, err := os.ReadFile(serviceFile)
		if err != nil {
			printYellow(i18n.T("install.service_missing"))
			fmt.Print(i18n.T("firsttime.platform_prompt"))
			choice, _ := readLine()
			platform, _, err = recoveryPlatformForService(nil, choice)
			if err != nil {
				printRed(err.Error())
				return
			}
		} else {
			var err error
			platform, _, err = recoveryPlatformForService(service, "")
			if err != nil {
				printRed(err.Error())
				return
			}
		}
		configExists = true
	}

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		return
	}

	if configExists {
		printGreen(i18n.T("install.config_exists"))
	} else {
		var err error
		platform, simplexPort, err = firstTimeSetup(destPath)
		if err != nil {
			return
		}
		if _, err := os.Stat(configPath); err != nil {
			printRed(i18n.T("setup.failed", err.Error()))
			return
		}
	}

	writeSystemdService(platform)
	deploySimplexService(platform, simplexPort)

	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = runCmdSilent("systemctl", "enable", serviceName)
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed(i18n.T("install.service_failed", err.Error()))
		return
	}

	printGreen(i18n.T("install.success"))
	printSkyBlue(i18n.T("install.manage_hint"))
}

// ======================== 无交互安装 (JSON / Key=Value) ========

func generateTOTPSecret(destPath string) string {
	printYellow(i18n.T("totp.generating"))
	output, err := runCmdOutputBytes(destPath, "--generate-totp-secret")
	if err != nil {
		printRed(i18n.T("totp.generate_failed", err.Error()))
		os.Exit(1)
	}
	rawSecret, err := extractBase32Secret(output)
	if err != nil {
		printRed(i18n.T("totp.parse_failed", err.Error()))
		os.Exit(1)
	}
	printYellow(i18n.T("totp.generated"))
	return string(rawSecret)
}

func runSetupCommand(destPath string, payload []byte) error {
	cmd := exec.Command(destPath, "--setup-stdin")
	cmd.Stdin = bytes.NewReader(payload)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

func runAegisSetup(destPath string, payload []byte) {
	printYellow(i18n.T("setup.configuring"))
	if err := runSetupCommand(destPath, payload); err != nil {
		printRed(i18n.T("setup.failed", err.Error()))
		os.Exit(1)
	}
}

func finishDeploy(platform string, simplexPort string) {
	writeSystemdService(platform)
	deploySimplexService(platform, simplexPort)
	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = runCmdSilent("systemctl", "enable", serviceName)
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed(i18n.T("install.service_failed", err.Error()))
		os.Exit(1)
	}
	printGreen(i18n.T("install.success"))
	printSkyBlue(i18n.T("install.manage_hint"))
}

func installFromStdin() {
	payload, err := io.ReadAll(os.Stdin)
	if err != nil {
		printRed(i18n.T("stdin.read_failed", err.Error()))
		os.Exit(1)
	}

	if !json.Valid(payload) {
		printRed(i18n.T("stdin.invalid_json"))
		os.Exit(1)
	}

	var inputData map[string]interface{}
	if err := json.Unmarshal(payload, &inputData); err != nil {
		printRed(i18n.T("stdin.parse_failed", err.Error()))
		os.Exit(1)
	}
	if token, _ := inputData["token"].(string); token != "" {
		adminID, ok := inputData["admin_id"].(string)
		if !ok || validateAdminID(adminID) != nil {
			printRed("admin_id 必须是有效的 i64")
			os.Exit(1)
		}
	}

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		os.Exit(1)
	}

	simplexPort, _ := inputData["simplex_port"].(string)
	// Discord 平台已移除，按「键存在」即拒绝（不只看非空值）：手写 payload 里带一个空的
	// discord_token 也必须失败，不能静默落到 tg 默认平台。
	if _, ok := inputData["discord_token"]; ok {
		printRed(i18n.T("install.discord_removed"))
		os.Exit(1)
	}
	// 组合由字段组合显式决定：token + simplex_port 即 tg-simplex。
	// 判定用「非空」而非「键存在」：显式传空的 simplex_port 不再被当成 SimpleX 部署。
	token, _ := inputData["token"].(string)
	matrixHS, _ := inputData["matrix_homeserver"].(string)
	platform, err := platformForNonInteractive(token != "", matrixHS != "", simplexPort != "")
	if err != nil {
		printRed(err.Error())
		os.Exit(1)
	}

	secret, hasSecret := inputData["totp_secret"].(string)
	if !hasSecret || secret == "" {
		secret = generateTOTPSecret(destPath)
		inputData["totp_secret"] = secret
		payload, err = json.Marshal(inputData)
		if err != nil {
			printRed(i18n.T("stdin.serialize_failed", err.Error()))
			os.Exit(1)
		}
	}

	runAegisSetup(destPath, payload)
	finishDeploy(platform, simplexPort)
}

type setupConfig struct {
	Token                 string
	AdminID               string
	TOTPSecret            string
	MatrixHS              string
	MatrixUser            string
	MatrixPassword        string
	MatrixRoom            string
	MatrixStorePassphrase string
	MatrixRecoveryKey     string
	SimplexPort           string
	SimplexAdminID        string
}

func validateAdminID(id string) error {
	if _, err := strconv.ParseInt(id, 10, 64); err != nil {
		return fmt.Errorf("admin_id 必须是有效的 i64: %w", err)
	}
	return nil
}

func parseKeyVal(data []byte) (*setupConfig, error) {
	s := strings.ReplaceAll(string(data), "\r\n", "\n")
	lines := strings.Split(s, "\n")
	cfg := &setupConfig{}
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		key, val, found := strings.Cut(line, "=")
		if !found {
			continue
		}
		key = strings.TrimSpace(key)
		val = strings.TrimSpace(val)
		switch key {
		case "token":
			cfg.Token = val
		case "admin_id":
			cfg.AdminID = val
		case "totp_secret":
			cfg.TOTPSecret = val
		case "matrix_homeserver":
			cfg.MatrixHS = val
		case "matrix_username":
			cfg.MatrixUser = val
		case "matrix_password":
			cfg.MatrixPassword = val
		case "matrix_room_id":
			cfg.MatrixRoom = val
		case "matrix_store_passphrase":
			cfg.MatrixStorePassphrase = val
		case "discord_token", "discord_admin_id":
			return nil, fmt.Errorf("%s", i18n.T("install.discord_removed"))
		case "matrix_recovery_key":
			cfg.MatrixRecoveryKey = val
		case "simplex_port":
			cfg.SimplexPort = val
		case "simplex_admin_id":
			cfg.SimplexAdminID = val
		default:
			printYellow(i18n.T("keyval.unknown_field", key))
		}
	}
	if cfg.Token == "" && cfg.MatrixHS == "" && cfg.SimplexPort == "" {
		return nil, fmt.Errorf("%s", missingPlatformFieldsMsg)
	}
	if cfg.Token != "" {
		if err := validateAdminID(cfg.AdminID); err != nil {
			return nil, err
		}
	}
	return cfg, nil
}

func installFromKeyVal() {
	data, err := io.ReadAll(os.Stdin)
	if err != nil {
		printRed(i18n.T("stdin.read_failed", err.Error()))
		os.Exit(1)
	}

	cfg, err := parseKeyVal(data)
	if err != nil {
		printRed(err.Error())
		os.Exit(1)
	}

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		os.Exit(1)
	}

	if cfg.TOTPSecret == "" {
		cfg.TOTPSecret = generateTOTPSecret(destPath)
	}

	// matrix_username 与 matrix_homeserver 任一存在即视为配置了 Matrix（沿用既有口径）。
	platform, err := platformForNonInteractive(
		cfg.Token != "",
		cfg.MatrixHS != "" || cfg.MatrixUser != "",
		cfg.SimplexPort != "",
	)
	if err != nil {
		printRed(err.Error())
		os.Exit(1)
	}

	payload := buildSetupPayload(
		[]byte(cfg.Token), []byte(cfg.AdminID), []byte(cfg.TOTPSecret),
		cfg.MatrixHS, cfg.MatrixUser, cfg.MatrixRoom, []byte(cfg.MatrixPassword), []byte(cfg.MatrixStorePassphrase),
		cfg.MatrixRecoveryKey,
		cfg.SimplexPort, cfg.SimplexAdminID,
	)

	runAegisSetup(destPath, payload)
	finishDeploy(platform, cfg.SimplexPort)
}

func platformSetupForChoice(choice string) (tg, matrix, simplex bool, err error) {
	switch choice {
	case "1":
		return true, false, false, nil
	case "2":
		return false, true, false, nil
	case "3":
		// 编号 3 原为 Discord：保留空洞，避免旧脚本静默落到别的平台。
		return false, false, false, fmt.Errorf("%s", i18n.T("install.discord_removed"))
	case "4":
		return true, true, false, nil
	case "5":
		return false, false, true, nil
	case "6":
		return true, false, true, nil
	default:
		return false, false, false, fmt.Errorf("invalid platform")
	}
}

func servicePlatformForSetup(tg, matrix, simplex bool) string {
	switch {
	case tg && simplex:
		return "tg-simplex"
	case simplex:
		return "simplex"
	case tg && matrix:
		return "tg-matrix"
	case tg:
		return "tg"
	case matrix:
		return "matrix"
	default:
		return ""
	}
}

// missingPlatformFieldsMsg 由非交互安装的两条路径与 parseKeyVal 共用同一条文案。
const missingPlatformFieldsMsg = "缺少必填字段: 至少需要配置 Telegram (token/admin_id)、Matrix (matrix_homeserver) 或 SimpleX (simplex_port/simplex_admin_id) 之一"

// platformForNonInteractive 由非交互安装（stdin JSON / key=value）的字段是否非空推导部署形态。
// 组合必须显式：只有 token 与 simplex_port 同时存在才是 tg-simplex；三个平台字段齐备
// 直接报错而不猜优先级，与 aegis 侧「配置歧义」的立场一致。
func platformForNonInteractive(hasToken, hasMatrix, hasSimplex bool) (string, error) {
	switch {
	case hasToken && hasMatrix && hasSimplex:
		return "", fmt.Errorf("%s", i18n.T("install.platform_ambiguous_three"))
	case hasToken && hasSimplex:
		return "tg-simplex", nil
	case hasToken && hasMatrix:
		return "tg-matrix", nil
	case hasToken:
		return "tg", nil
	case hasSimplex:
		return "simplex", nil
	case hasMatrix:
		return "matrix", nil
	default:
		return "", fmt.Errorf("%s", missingPlatformFieldsMsg)
	}
}

func firstTimeSetup(binaryPath string) (string, string, error) {
	printSkyBlue(i18n.T("firsttime.title"))

	enableTG, enableMatrix, enableSimplex, err := selectDeploymentPlatforms()
	if err != nil {
		printRed(err.Error())
		return "", "", err
	}

	var botTokenEnclave *memguard.Enclave
	var adminIDEnclave *memguard.Enclave

	if enableTG {
		printSkyBlue(i18n.T("firsttime.section_tg"))
		printYellow(i18n.T("firsttime.tg_help_howto"))
		printYellow(i18n.T("firsttime.tg_help_step1"))
		printYellow(i18n.T("firsttime.tg_help_step2"))
		printYellow(i18n.T("firsttime.tg_help_step3"))
		printYellow(i18n.T("firsttime.tg_help_format"))
		fmt.Println()

		botTokenEnclave = readSecureInput(i18n.T("firsttime.tg_prompt"))

		printYellow(i18n.T("firsttime.admin_help_howto"))
		printYellow(i18n.T("firsttime.admin_help_step1"))
		printYellow(i18n.T("firsttime.admin_help_step2"))
		printYellow(i18n.T("firsttime.admin_help_step3"))
		printYellow(i18n.T("firsttime.admin_help_format"))
		fmt.Println()

		adminIDEnclave = readSecureInput(i18n.T("firsttime.admin_prompt"))
	}

	var totpSecretEnclave *memguard.Enclave

	if enableTG {
		totpSecretOutput, err := runCmdOutputBytes(binaryPath, "--generate-totp-secret")
		if err != nil {
			printRed(i18n.T("totp.generate_failed", err.Error()))
			return "", "", err
		}
		defer zeroBytes(totpSecretOutput)

		totpSecretRaw, err := extractBase32Secret(totpSecretOutput)
		if err != nil {
			printRed(i18n.T("totp.parse_failed", err.Error()))
			return "", "", err
		}
		defer zeroBytes(totpSecretRaw)

		totpSecretEnclave = memguard.NewEnclave(totpSecretRaw)

		totpSecretBuffer, _ := totpSecretEnclave.Open()
		otpauthURL := buildOtpAuthURL(totpSecretBuffer.Bytes())
		defer zeroBytes(otpauthURL)

		printYellow(i18n.T("firsttime.totp_section"))
		writeLine(i18n.T("firsttime.totp_key_label"), totpSecretBuffer.Bytes())

		if _, err := exec.LookPath("qrencode"); err == nil {
			printYellow(i18n.T("firsttime.totp_qr_scan"))
			cmd := exec.Command("qrencode", "-t", "ANSIUTF8")
			cmd.Stdout = os.Stdout
			cmd.Stderr = os.Stderr
			cmd.Stdin = bytes.NewReader(otpauthURL)
			_ = cmd.Run()
		} else {
			printYellow(i18n.T("firsttime.totp_installing_qr"))
			if err := runCmdSilent("apt-get", "install", "-y", "qrencode"); err == nil {
				printYellow(i18n.T("firsttime.totp_qr_scan"))
				cmd := exec.Command("qrencode", "-t", "ANSIUTF8")
				cmd.Stdout = os.Stdout
				cmd.Stderr = os.Stderr
				cmd.Stdin = bytes.NewReader(otpauthURL)
				_ = cmd.Run()
			} else {
				printYellow(i18n.T("firsttime.totp_no_qr"))
			}
		}

		writeLine(i18n.T("firsttime.totp_manual_url"), otpauthURL)
		printYellow(i18n.T("firsttime.totp_clear_hint"))
		printYellow(i18n.T("firsttime.totp_separator"))

		totpSecretBuffer.Destroy()
	}

	var matrixHS, matrixUser, matrixRoom, matrixRecoveryKey string
	var matrixPassEnclave *memguard.Enclave

	if enableMatrix {
		printSkyBlue(i18n.T("firsttime.matrix_section"))
		printYellow(i18n.T("firsttime.matrix_desc1"))
		printYellow(i18n.T("firsttime.matrix_desc2"))
		printYellow(i18n.T("firsttime.matrix_desc3"))
		printYellow(i18n.T("firsttime.matrix_user_title"))
		printYellow(i18n.T("firsttime.matrix_user_desc"))
		printYellow(i18n.T("firsttime.matrix_user_format"))
		matrixUser = readSecureInputStr(i18n.T("firsttime.matrix_mxid_prompt"))
		matrixUser, matrixHS, err = selectMatrixHomeserver(matrixUser)
		if err != nil {
			return "", "", err
		}

		matrixPassEnclave = readSecureInput(i18n.T("firsttime.matrix_pass_prompt"))

		printYellow(i18n.T("firsttime.matrix_room_title"))
		printYellow(i18n.T("firsttime.matrix_room_step1"))
		printYellow(i18n.T("firsttime.matrix_room_step2"))
		printYellow(i18n.T("firsttime.matrix_room_step3"))
		printYellow(i18n.T("firsttime.matrix_room_format"))
		printYellow(i18n.T("firsttime.matrix_room_warn"))
		matrixRoom = readSecureInputStr(i18n.T("firsttime.matrix_room_prompt"))

		// ── Matrix Recovery Key ──
		printYellow(i18n.T("firsttime.matrix_recovery_title"))
		printYellow(i18n.T("firsttime.matrix_recovery_desc1"))
		printYellow(i18n.T("firsttime.matrix_recovery_desc2"))
		matrixRecoveryKey = readSecureInputStr(i18n.T("firsttime.matrix_recovery_prompt"))
	}

	// ── SimpleX section ──
	var simplexPort, simplexAdminID string
	if enableSimplex {
		printSkyBlue(i18n.T("firsttime.simplex_section"))
		printYellow(i18n.T("firsttime.simplex_desc1"))
		printYellow(i18n.T("firsttime.simplex_desc2"))
		printYellow(i18n.T("firsttime.simplex_desc3"))
		printYellow(i18n.T("firsttime.simplex_port_title"))
		simplexPort = readSecureInputStr(i18n.T("firsttime.simplex_port_prompt"))
		if simplexPort == "" {
			simplexPort = defaultSimplexPort
		}

		printYellow(i18n.T("firsttime.simplex_admin_title"))
		printYellow(i18n.T("firsttime.simplex_admin_help_step1"))
		printYellow(i18n.T("firsttime.simplex_admin_help_step2"))
		printYellow(i18n.T("firsttime.simplex_admin_help_format"))
		simplexAdminID = readSecureInputStr(i18n.T("firsttime.simplex_admin_prompt"))
	}

	var bTokenBytes, aIDBytes, tSecretBytes []byte
	var bTokenBuf, aIDBuf, tSecretBuf *memguard.LockedBuffer
	if botTokenEnclave != nil {
		bTokenBuf, _ = botTokenEnclave.Open()
		defer bTokenBuf.Destroy()
		bTokenBytes = bTokenBuf.Bytes()
	}
	if adminIDEnclave != nil {
		aIDBuf, _ = adminIDEnclave.Open()
		defer aIDBuf.Destroy()
		aIDBytes = aIDBuf.Bytes()
		if err := validateAdminID(string(aIDBytes)); err != nil {
			printRed(err.Error())
			return "", "", err
		}
	}
	if totpSecretEnclave != nil {
		tSecretBuf, _ = totpSecretEnclave.Open()
		defer tSecretBuf.Destroy()
		tSecretBytes = tSecretBuf.Bytes()
	}

	var mPassBuf *memguard.LockedBuffer
	var mPassBytes []byte
	var matrixStorePassphrase string
	if matrixPassEnclave != nil {
		mPassBuf, _ = matrixPassEnclave.Open()
		mPassBytes = mPassBuf.Bytes()
		matrixStorePassphrase = randomString(32)
	}

	setupPayload := buildSetupPayload(
		bTokenBytes, aIDBytes, tSecretBytes,
		matrixHS, matrixUser, matrixRoom, mPassBytes, []byte(matrixStorePassphrase),
		matrixRecoveryKey,
		simplexPort, simplexAdminID,
	)
	defer zeroBytes(setupPayload)

	if mPassBuf != nil {
		mPassBuf.Destroy()
	}
	if err := runSetupCommand(binaryPath, setupPayload); err != nil {
		printRed(i18n.T("setup.failed", err.Error()))
		return "", "", err
	}

	return servicePlatformForSetup(enableTG, enableMatrix, enableSimplex), simplexPort, nil
}

// readSecureInput 安全地从终端读取输入，直接返回加密的 Enclave，避免产生明文 string 垃圾
func readSecureInput(prompt string) *memguard.Enclave {
	fmt.Print(prompt)

	// 分配一块安全的锁定内存
	b := memguard.NewBuffer(512)
	defer b.Destroy() // 确保函数返回前销毁明文缓冲

	n, err := os.Stdin.Read(b.Bytes())
	if err != nil {
		printRed(i18n.T("input.read_failed", err.Error()))
		memguard.Purge()
		os.Exit(1)
	}

	// 截断换行符并保留实际输入
	actualData := b.Bytes()[:n]
	if len(actualData) > 0 && actualData[len(actualData)-1] == '\n' {
		actualData = actualData[:len(actualData)-1]
	}
	if len(actualData) > 0 && actualData[len(actualData)-1] == '\r' {
		actualData = actualData[:len(actualData)-1]
	}

	// 密封到 Enclave 并返回
	return memguard.NewEnclave(actualData)
}

func platformFromService(service []byte) string {
	switch {
	case bytes.Contains(service, []byte("--matrix")):
		return "matrix"
	case bytes.Contains(service, []byte("--tg-simplex")):
		return "tg-simplex"
	case bytes.Contains(service, []byte("--simplex")):
		return "simplex"
	case bytes.Contains(service, []byte("--all")):
		return "tg-matrix"
	default:
		return "tg"
	}
}

func recoveryPlatformForService(service []byte, choice string) (string, bool, error) {
	if len(service) > 0 {
		// 既有单元带 --discord：硬失败，否则会退化成 tg 默认单元静默换平台。
		if bytes.Contains(service, []byte("--discord")) {
			return "", false, fmt.Errorf("%s", i18n.T("install.discord_removed"))
		}
		return platformFromService(service), false, nil
	}
	tg, matrix, simplex, err := platformSetupForChoice(choice)
	if err != nil {
		return "", false, err
	}
	return servicePlatformForSetup(tg, matrix, simplex), true, nil
}

// platformFlagFor 把平台标识映射为 aegis 的启动参数。
func platformFlagFor(platform string) string {
	switch platform {
	case "matrix":
		return "--matrix"
	case "simplex":
		return "--simplex"
	case "tg-simplex":
		return "--tg-simplex"
	case "tg-matrix":
		return "--all"
	}
	return ""
}

func writeSystemdService(platform string) {
	platformFlag := platformFlagFor(platform)
	descName := "WWPS Telegram Bot"
	switch platform {
	case "matrix":
		descName = "WWPS Matrix Bot"
	case "simplex":
		descName = "WWPS SimpleX Bot"
	case "tg-simplex":
		descName = "WWPS Telegram + SimpleX Bot"
	case "tg-matrix":
		descName = "WWPS Telegram + Matrix Bot"
	}
	execArgs := platformFlag
	if execArgs != "" {
		execArgs = " " + execArgs
	}
	content := `[Unit]
Description=` + descName + `
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=` + installDir + `
ExecStart=` + filepath.Join(installDir, binaryName) + execArgs + `
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
`
	if err := os.WriteFile(serviceFile, []byte(content), 0o644); err != nil {
		printRed("写入 systemd 服务文件失败: " + err.Error())
	}
}

// ======================== 卸载 ==============================

func uninstallAegis() {
	printYellow(i18n.T("uninstall.confirm"))
	fmt.Print(i18n.T("uninstall.confirm_prompt"))
	confirm, _ := readLine()

	if confirm != "y" {
		printGreen(i18n.T("uninstall.cancelled"))
		return
	}

	for _, service := range uninstallServices {
		_ = runCmdSilent("systemctl", "stop", service)
		_ = runCmdSilent("systemctl", "disable", service)
	}
	_ = runCmdSilent("rc-service", "wwps-core", "stop")
	_ = runCmdSilent("rc-update", "del", "wwps-core", "default")

	for _, path := range uninstallPaths {
		_ = os.RemoveAll(path)
	}
	_ = runCmdSilent("systemctl", "daemon-reload")

	printGreen(i18n.T("uninstall.done"))
}

// ======================== 状态 ==============================

func showStatus() {
	printSkyBlue(i18n.T("status.title"))

	binPath := filepath.Join(installDir, binaryName)
	if _, err := os.Stat(binPath); err == nil {
		printGreen(i18n.T("status.binary_installed"))
	} else {
		printYellow(i18n.T("status.binary_missing"))
	}

	if err := runCmdSilent("systemctl", "is-active", "--quiet", serviceName); err == nil {
		printGreen(i18n.T("status.service_running"))
	} else if runCmdSilent("systemctl", "is-enabled", "--quiet", serviceName) == nil {
		printYellow(i18n.T("status.service_stopped"))
	} else {
		printYellow(i18n.T("status.service_not_installed"))
	}

	configPath := filepath.Join(installDir, "config.enc")
	if _, err := os.Stat(configPath); err == nil {
		printGreen(i18n.T("status.config_ready"))
	} else {
		printYellow(i18n.T("status.config_missing"))
	}

	fmt.Println()
}

// ======================== 主入口 =============================

func main() {
	// 启用 memguard 安全退出机制：捕获中断信号 (Ctrl+C) 并确保清空加密内存
	memguard.CatchInterrupt()
	defer memguard.Purge()
	// 如果检测到一些不可抗拒崩溃，这里也拦截一下
	defer func() {
		if r := recover(); r != nil {
			memguard.Purge()
			fmt.Println(i18n.T("crash.cleaned", r))
			os.Exit(1)
		}
	}()

	disableCoreDumps()
	checkRoot()
	_ = checkArch()

	if len(os.Args) > 1 && os.Args[1] == "--setup-stdin" {
		i18n.InitLang(false)
		installFromStdin()
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "--setup-keyval" {
		i18n.InitLang(false)
		installFromKeyVal()
		return
	}

	i18n.InitLang(true)
	printBanner()
	showStatus()

	printYellow(i18n.T("menu.install"))
	printYellow(i18n.T("menu.uninstall"))
	printYellow(i18n.T("menu.exit"))

	fmt.Print(i18n.T("menu.prompt"))
	choice, _ := readLine()

	switch choice {
	case "1":
		installAegis()
	case "2":
		uninstallAegis()
	case "0":
		os.Exit(0)
	default:
		printRed(i18n.T("menu.invalid"))
		os.Exit(1)
	}
}
