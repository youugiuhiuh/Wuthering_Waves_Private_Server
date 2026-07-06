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
	"os"
	"os/exec"
	"path/filepath"
	"regexp"
	"runtime"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/NicholasDewar/Wuthering_Waves_Private_Server/go/installer/i18n"

	"github.com/awnumar/memguard"
	"golang.org/x/sys/unix"
)

const (
	version     = "3.2.10"
	installDir  = "/etc/wwps/aegis"
	binaryName  = "aegis"
	serviceName = "wwps-aegis"
	serviceFile = "/etc/systemd/system/wwps-aegis.service"
)

type releaseRepo struct {
	Owner string
	Name  string
}

var defaultReleaseRepositories = []releaseRepo{
	{Owner: "NicholasDewar", Name: "Wuthering_Waves_Private_Server"},
	{Owner: "youugiuhiuh", Name: "Wuthering_Waves_Private_Server"},
}

// releaseAPIBases: 按顺序尝试的 Release API 根地址，可通过 AEGIS_RELEASE_MIRRORS 覆盖。
var releaseAPIBases = []string{
	"https://api.github.com",
}

func init() {
	if s := os.Getenv("AEGIS_RELEASE_MIRRORS"); s != "" {
		bases := strings.Split(s, ",")
		for i := range bases {
			bases[i] = strings.TrimSpace(bases[i])
		}
		if len(bases) > 0 && bases[0] != "" {
			releaseAPIBases = bases
		}
	}
}

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

func runCmd(name string, args ...string) error {
	cmd := exec.Command(name, args...)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	return cmd.Run()
}

func runCmdSilent(name string, args ...string) error {
	cmd := exec.Command(name, args...)
	return cmd.Run()
}

func runCmdOutput(name string, args ...string) (string, error) {
	cmd := exec.Command(name, args...)
	out, err := cmd.Output()
	return strings.TrimSpace(string(out)), err
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

func buildSetupPayload(token, adminID, totpSecret []byte, matrixHS, matrixUser, matrixRoom string, matrixPass, matrixStorePassphrase []byte) []byte {
	payload := make([]byte, 0, len(token)+len(adminID)+len(totpSecret)+64)
	payload = append(payload, '{')
	payload = append(payload, []byte(`"token":`)...)
	payload = appendJSONEscaped(payload, token)
	payload = append(payload, ',')
	payload = append(payload, []byte(`"admin_id":`)...)
	payload = appendJSONEscaped(payload, adminID)
	payload = append(payload, ',')
	payload = append(payload, []byte(`"totp_secret":`)...)
	payload = appendJSONEscaped(payload, totpSecret)

	if matrixHS != "" {
		payload = append(payload, ',')
		payload = append(payload, []byte(`"matrix_homeserver":`)...)
		payload = appendJSONEscaped(payload, []byte(matrixHS))
	}
	if matrixUser != "" {
		payload = append(payload, ',')
		payload = append(payload, []byte(`"matrix_username":`)...)
		payload = appendJSONEscaped(payload, []byte(matrixUser))
	}
	if len(matrixPass) > 0 {
		payload = append(payload, ',')
		payload = append(payload, []byte(`"matrix_password":`)...)
		payload = appendJSONEscaped(payload, matrixPass)
	}
	if matrixRoom != "" {
		payload = append(payload, ',')
		payload = append(payload, []byte(`"matrix_room_id":`)...)
		payload = appendJSONEscaped(payload, []byte(matrixRoom))
	}
	if len(matrixStorePassphrase) > 0 {
		payload = append(payload, ',')
		payload = append(payload, []byte(`"matrix_store_passphrase":`)...)
		payload = appendJSONEscaped(payload, matrixStorePassphrase)
	}

	payload = append(payload, '}')
	return payload
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
		for _, base := range releaseAPIBases {
			base = strings.TrimSuffix(base, "/")
			apiURL := base + apiPath
			resp, err := client.Get(apiURL)
			if err != nil {
				errors = append(errors, fmt.Sprintf("%s/%s via %s: %v", repo.Owner, repo.Name, base, err))
				continue
			}
			if resp.StatusCode != http.StatusOK {
				resp.Body.Close()
				errors = append(errors, fmt.Sprintf("%s/%s via %s 返回状态码: %d", repo.Owner, repo.Name, base, resp.StatusCode))
				continue
			}
			body, err := io.ReadAll(resp.Body)
			resp.Body.Close()
			if err != nil {
				errors = append(errors, fmt.Sprintf("%s/%s via %s 读取失败: %v", repo.Owner, repo.Name, base, err))
				continue
			}
			var release latestRelease
			if err := json.Unmarshal(body, &release); err != nil {
				errors = append(errors, fmt.Sprintf("%s/%s via %s 解析 JSON 失败: %v", repo.Owner, repo.Name, base, err))
				continue
			}
			if release.TagName == "" {
				errors = append(errors, fmt.Sprintf("%s/%s via %s release 缺少 tag_name", repo.Owner, repo.Name, base))
				continue
			}
			return &release, nil
		}
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
			info, err := verifyMinisign(binaryPath, sigPath, minisignPublicKeys)
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
			if !strings.HasPrefix(gotVersion, expectedVersion) {
				printRed(i18n.T("minisign.version_mismatch", expectedVersion, gotVersion))
				return ""
			}
			if gotAsset != binaryName {
				printRed(i18n.T("minisign.asset_mismatch", binaryName, gotAsset))
				return ""
			}
			printGreen(i18n.T("minisign.verify_ok"))
			printYellow(i18n.T("minisign.trusted_comment", info.TrustedComment))
			minisigPassed = true
		}
	}
	if !minisigPassed {
		printYellow(i18n.T("minisign.skipped"))
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

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		return
	}

	configPath := filepath.Join(installDir, "config.enc")
	if _, err := os.Stat(configPath); err == nil {
		printGreen(i18n.T("install.config_exists"))
	} else {
		firstTimeSetup(destPath)
	}

	writeSystemdService()

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

func runAegisSetup(destPath string, payload []byte) {
	printYellow(i18n.T("setup.configuring"))
	cmd := exec.Command(destPath, "--setup-stdin")
	cmd.Stdin = bytes.NewReader(payload)
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	if err := cmd.Run(); err != nil {
		printRed(i18n.T("setup.failed", err.Error()))
		os.Exit(1)
	}
}

func finishDeploy() {
	writeSystemdService()
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

	destPath := downloadAndDeployAegis()
	if destPath == "" {
		os.Exit(1)
	}

	var inputData map[string]interface{}
	if err := json.Unmarshal(payload, &inputData); err != nil {
		printRed(i18n.T("stdin.parse_failed", err.Error()))
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
	finishDeploy()
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
		default:
			printYellow(i18n.T("keyval.unknown_field", key))
		}
	}
	if cfg.Token == "" || cfg.AdminID == "" {
		return nil, fmt.Errorf("缺少必填字段: token, admin_id")
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

	payload := buildSetupPayload(
		[]byte(cfg.Token), []byte(cfg.AdminID), []byte(cfg.TOTPSecret),
		cfg.MatrixHS, cfg.MatrixUser, cfg.MatrixRoom, []byte(cfg.MatrixPassword), []byte(cfg.MatrixStorePassphrase),
	)

	runAegisSetup(destPath, payload)
	finishDeploy()
}

func firstTimeSetup(binaryPath string) {
	printSkyBlue(i18n.T("firsttime.title"))

	printSkyBlue(i18n.T("firsttime.section_tg"))
	printYellow(i18n.T("firsttime.tg_help_howto"))
	printYellow(i18n.T("firsttime.tg_help_step1"))
	printYellow(i18n.T("firsttime.tg_help_step2"))
	printYellow(i18n.T("firsttime.tg_help_step3"))
	printYellow(i18n.T("firsttime.tg_help_format"))
	fmt.Println()

	botTokenEnclave := readSecureInput(i18n.T("firsttime.tg_prompt"))

	printYellow(i18n.T("firsttime.admin_help_howto"))
	printYellow(i18n.T("firsttime.admin_help_step1"))
	printYellow(i18n.T("firsttime.admin_help_step2"))
	printYellow(i18n.T("firsttime.admin_help_step3"))
	printYellow(i18n.T("firsttime.admin_help_format"))
	fmt.Println()

	adminIDEnclave := readSecureInput(i18n.T("firsttime.admin_prompt"))

	totpSecretOutput, err := runCmdOutputBytes(binaryPath, "--generate-totp-secret")
	if err != nil {
		printRed(i18n.T("totp.generate_failed", err.Error()))
		return
	}
	defer zeroBytes(totpSecretOutput)

	totpSecretRaw, err := extractBase32Secret(totpSecretOutput)
	if err != nil {
		printRed(i18n.T("totp.parse_failed", err.Error()))
		return
	}
	defer zeroBytes(totpSecretRaw)

	totpSecretEnclave := memguard.NewEnclave(totpSecretRaw)

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

	printSkyBlue(i18n.T("firsttime.matrix_section"))
	printYellow(i18n.T("firsttime.matrix_desc1"))
	printYellow(i18n.T("firsttime.matrix_desc2"))
	printYellow(i18n.T("firsttime.matrix_desc3"))
	fmt.Print(i18n.T("firsttime.matrix_prompt_yn"))
	setupMatrix, _ := readLine()

	var matrixHS, matrixUser, matrixRoom string
	var matrixPassEnclave *memguard.Enclave

	if setupMatrix == "y" || setupMatrix == "Y" {
		printYellow(i18n.T("firsttime.matrix_hs_title"))
		printYellow(i18n.T("firsttime.matrix_hs_default"))
		printYellow(i18n.T("firsttime.matrix_hs_custom"))
		fmt.Print(i18n.T("firsttime.matrix_hs_prompt"))
		matrixHS, _ = readLine()
		if matrixHS == "" {
			matrixHS = "https://matrix.org"
		}

		printYellow(i18n.T("firsttime.matrix_user_title"))
		printYellow(i18n.T("firsttime.matrix_user_desc"))
		printYellow(i18n.T("firsttime.matrix_user_format"))
		matrixUser = readSecureInputStr(i18n.T("firsttime.matrix_user_prompt"))

		matrixPassEnclave = readSecureInput(i18n.T("firsttime.matrix_pass_prompt"))

		printYellow(i18n.T("firsttime.matrix_room_title"))
		printYellow(i18n.T("firsttime.matrix_room_step1"))
		printYellow(i18n.T("firsttime.matrix_room_step2"))
		printYellow(i18n.T("firsttime.matrix_room_step3"))
		printYellow(i18n.T("firsttime.matrix_room_format"))
		printYellow(i18n.T("firsttime.matrix_room_warn"))
		matrixRoom = readSecureInputStr(i18n.T("firsttime.matrix_room_prompt"))
	}

	bTokenBuf, _ := botTokenEnclave.Open()
	aIDBuf, _ := adminIDEnclave.Open()
	tSecretBuf, _ := totpSecretEnclave.Open()

	var mPassBuf *memguard.LockedBuffer
	var mPassBytes []byte
	var matrixStorePassphrase string
	if matrixPassEnclave != nil {
		mPassBuf, _ = matrixPassEnclave.Open()
		mPassBytes = mPassBuf.Bytes()
		matrixStorePassphrase = randomString(32)
	}

	setupPayload := buildSetupPayload(
		bTokenBuf.Bytes(), aIDBuf.Bytes(), tSecretBuf.Bytes(),
		matrixHS, matrixUser, matrixRoom, mPassBytes, []byte(matrixStorePassphrase),
	)
	defer zeroBytes(setupPayload)

	if mPassBuf != nil {
		mPassBuf.Destroy()
	}
	cmd := exec.Command(binaryPath, "--setup-stdin")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = bytes.NewReader(setupPayload)

	if err := cmd.Run(); err != nil {
		printRed(i18n.T("setup.failed", err.Error()))
	}

	bTokenBuf.Destroy()
	aIDBuf.Destroy()
	tSecretBuf.Destroy()
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

func writeSystemdService() {
	content := `[Unit]
Description=WWPS Telegram Bot
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=` + installDir + `
ExecStart=` + filepath.Join(installDir, binaryName) + `
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

	_ = runCmdSilent("systemctl", "stop", serviceName)
	_ = runCmdSilent("systemctl", "disable", serviceName)
	_ = os.Remove(serviceFile)
	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = os.RemoveAll(installDir)

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
