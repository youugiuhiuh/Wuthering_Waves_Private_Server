package main

import (
	"bytes"
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

	"github.com/awnumar/memguard"
	"golang.org/x/sys/unix"
)

const (
	version     = "v0.3.4"
	repoOwner   = "NicholasDewar"
	repoName    = "Wuthering_Waves_Private_Server"
	installDir  = "/etc/wwps/tgbot"
	binaryName  = "tgbot"
	serviceName = "wwps-tgbot"
	serviceFile = "/etc/systemd/system/wwps-tgbot.service"
)

// releaseAPIBases: 按顺序尝试的 Release API 根地址（支持 GitHub / Codeberg / Gitea 等兼容 API）
var releaseAPIBases = []string{
	"https://api.github.com",
	"https://codeberg.org/api/v1",
	"https://gitea.com/api/v1",
}

func init() {
	if s := os.Getenv("TGBOT_RELEASE_MIRRORS"); s != "" {
		bases := strings.Split(s, ",")
		for i := range bases {
			bases[i] = strings.TrimSpace(bases[i])
		}
		if len(bases) > 0 && bases[0] != "" {
			releaseAPIBases = bases
		}
	}
}

type releaseAsset struct {
	Name               string `json:"name"`
	BrowserDownloadURL string `json:"browser_download_url"`
	URL                string `json:"url"` // Gitea/Codeberg 等可能用 url
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
	printGreen("WWPS TG Bot 管理工具")
	printGreen("当前版本: " + version)
	printGreen("Release 源: GitHub / Codeberg / Gitea 等 (可设 TGBOT_RELEASE_MIRRORS)")
	printSkyBlue("所有管理功能请通过 Telegram Bot 完成")
	printRed("==============================================================")
}

// ======================== 系统检测 ===========================

func checkRoot() {
	if os.Getuid() != 0 {
		printRed("请使用 root 用户运行此程序")
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
		printRed("不支持的 CPU 架构: " + arch)
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
			if b < 0x20 {
				dst = append(dst, '\\', 'u', '0', '0', "0123456789abcdef"[b>>4], "0123456789abcdef"[b&0x0f])
			} else {
				dst = append(dst, b)
			}
		}
	}
	dst = append(dst, '"')
	return dst
}

func buildSetupPayload(token, adminID, totpSecret []byte) []byte {
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
		printYellow("警告: 禁用 core dump 失败: " + err.Error())
	}
	if err := unix.Prctl(unix.PR_SET_DUMPABLE, 0, 0, 0, 0); err != nil {
		printYellow("警告: 设置进程不可转储失败: " + err.Error())
	}
}

// ======================== 下载和校验 =========================

func newHTTPClient(timeout time.Duration) *http.Client {
	return &http.Client{Timeout: timeout}
}

func downloadFile(client *http.Client, url, dest string) error {
	printYellow("正在下载: " + url)

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

	printGreen(fmt.Sprintf("✓ 下载完成 (%d bytes)", written))
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
	apiPath := fmt.Sprintf("/repos/%s/%s/releases/latest", repoOwner, repoName)
	var lastErr error
	for _, base := range releaseAPIBases {
		base = strings.TrimSuffix(base, "/")
		apiURL := base + apiPath
		resp, err := client.Get(apiURL)
		if err != nil {
			lastErr = fmt.Errorf("%s: %w", base, err)
			continue
		}
		if resp.StatusCode != http.StatusOK {
			resp.Body.Close()
			lastErr = fmt.Errorf("%s 返回状态码: %d", base, resp.StatusCode)
			continue
		}
		body, err := io.ReadAll(resp.Body)
		resp.Body.Close()
		if err != nil {
			lastErr = err
			continue
		}
		var release latestRelease
		if err := json.Unmarshal(body, &release); err != nil {
			lastErr = fmt.Errorf("%s 解析 JSON 失败: %w", base, err)
			continue
		}
		if release.TagName == "" {
			lastErr = fmt.Errorf("%s release 缺少 tag_name", base)
			continue
		}
		return &release, nil
	}
	if lastErr != nil {
		return nil, fmt.Errorf("所有 Release 源均失败: %w", lastErr)
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

// assetDownloadURL 返回该 asset 的下载地址（兼容 GitHub browser_download_url 与 Gitea/Codeberg url）
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

	printYellow("SHA-256: " + actual)
	if subtle.ConstantTimeCompare([]byte(strings.ToLower(actual)), []byte(strings.ToLower(expected))) != 1 {
		return fmt.Errorf("SHA-256 不匹配: expected %s, got %s", expected, actual)
	}
	return nil
}

// ======================== 安装 ==============================

func installTGBot() {
	printSkyBlue("\n开始安装/更新 TG Bot...")

	release, err := getLatestReleaseInfo()
	if err != nil {
		printRed("获取最新版本信息失败: " + err.Error())
		return
	}

	ver := release.TagName
	printYellow("目标版本: " + ver)

	// 下载
	tmpDir, err := os.MkdirTemp("", "wwps-installer-*")
	if err != nil {
		printRed("创建临时目录失败: " + err.Error())
		return
	}
	defer os.RemoveAll(tmpDir)

	binaryPath := filepath.Join(tmpDir, binaryName)
	fallbackDownload := fmt.Sprintf("https://github.com/%s/%s/releases/download/%s/%s", repoOwner, repoName, ver, binaryName)
	asset := findAsset(release, binaryName)
	downloadURL := fallbackDownload
	if asset != nil {
		if u := assetDownloadURL(asset, fallbackDownload); u != "" {
			downloadURL = u
		}
	}

	if err := downloadFile(newHTTPClient(10*time.Minute), downloadURL, binaryPath); err != nil {
		printRed("下载失败: " + err.Error())
		return
	}

	// 校验文件存在且非空
	info, err := os.Stat(binaryPath)
	if err != nil || info.Size() == 0 {
		printRed("下载的文件无效")
		return
	}

	expectedHash, err := findExpectedSHA256(release, binaryName)
	if err != nil {
		printRed("获取可信 SHA-256 失败: " + err.Error())
		return
	}
	if err := verifySHA256(binaryPath, expectedHash); err != nil {
		printRed("二进制校验失败: " + err.Error())
		return
	}

	// 部署
	if err := os.MkdirAll(installDir, 0o755); err != nil {
		printRed("创建安装目录失败: " + err.Error())
		return
	}

	_ = runCmdSilent("systemctl", "stop", serviceName)

	destPath := filepath.Join(installDir, binaryName)
	src, err := os.Open(binaryPath)
	if err != nil {
		printRed("读取二进制失败: " + err.Error())
		return
	}
	defer src.Close()

	dst, err := os.OpenFile(destPath, os.O_CREATE|os.O_WRONLY|os.O_TRUNC, 0o755)
	if err != nil {
		printRed("写入安装目录失败: " + err.Error())
		return
	}
	defer dst.Close()

	if _, err := io.Copy(dst, src); err != nil {
		printRed("复制二进制失败: " + err.Error())
		return
	}
	dst.Close()
	src.Close()

	printGreen("✓ TG Bot 二进制文件部署完成")

	// 首次配置
	configPath := filepath.Join(installDir, "config.enc")
	if _, err := os.Stat(configPath); err == nil {
		printGreen("\n检测到已存在配置文件，跳过初始化设置。")
	} else {
		firstTimeSetup(destPath)
	}

	// systemd 服务
	writeSystemdService()

	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = runCmdSilent("systemctl", "enable", serviceName)
	if err := runCmdSilent("systemctl", "restart", serviceName); err != nil {
		printRed("启动服务失败: " + err.Error())
		return
	}

	printGreen("\n✅ TG Bot 已成功安装并启动！")
	printSkyBlue("请前往 Telegram 与 Bot 对话进行管理。")
}

func firstTimeSetup(binaryPath string) {
	printSkyBlue("\n首次安装，开始配置 TG Bot...")

	botTokenEnclave := readSecureInput("请输入 TG Bot Token: ")
	adminIDEnclave := readSecureInput("请输入管理员 ID (TG User ID): ")

	// 生成 TOTP 密钥
	totpSecretOutput, err := runCmdOutputBytes(binaryPath, "--generate-totp-secret")
	if err != nil {
		printRed("生成 TOTP 密钥失败: " + err.Error())
		return
	}
	defer zeroBytes(totpSecretOutput)

	totpSecretRaw, err := extractBase32Secret(totpSecretOutput)
	if err != nil {
		printRed("解析 TOTP 密钥失败: " + err.Error())
		return
	}
	defer zeroBytes(totpSecretRaw)

	// 立即将 TOTP 秘密放入 Enclave 加密
	totpSecretEnclave := memguard.NewEnclave(totpSecretRaw)

	// 在内存中短暂解密用于展示和生成二维码
	totpSecretBuffer, _ := totpSecretEnclave.Open()
	otpauthURL := buildOtpAuthURL(totpSecretBuffer.Bytes())
	defer zeroBytes(otpauthURL)

	printYellow("\n========== 重要: TOTP 绑定 ==========")
	writeLine("您的 TOTP 密钥: ", totpSecretBuffer.Bytes())

	// 尝试显示二维码
	if _, err := exec.LookPath("qrencode"); err == nil {
		printYellow("扫描二维码绑定 (请使用支持 SHA512 的 TOTP 客户端):")
		cmd := exec.Command("qrencode", "-t", "ANSIUTF8")
		cmd.Stdout = os.Stdout
		cmd.Stderr = os.Stderr
		cmd.Stdin = bytes.NewReader(otpauthURL)
		_ = cmd.Run()
	} else {
		printYellow("提示: 安装 qrencode 可显示二维码")
	}

	writeLine("手动添加链接: ", otpauthURL)
	printYellow("⚠ 绑定完成后请尽快清屏/关闭终端")
	printYellow("====================================\n")

	// 销毁用于展示的明文 Buffer
	totpSecretBuffer.Destroy()

	// 执行 setup 时，短暂解密 Token 和 AdminID (这里仍有 /proc/cmdline 的极短暂暴露风险，后续优化可以在子进程 stdin 传递)
	bTokenBuf, _ := botTokenEnclave.Open()
	aIDBuf, _ := adminIDEnclave.Open()
	tSecretBuf, _ := totpSecretEnclave.Open()

	setupPayload := buildSetupPayload(bTokenBuf.Bytes(), aIDBuf.Bytes(), tSecretBuf.Bytes())
	defer zeroBytes(setupPayload)

	cmd := exec.Command(binaryPath, "--setup-stdin")
	cmd.Stdout = os.Stdout
	cmd.Stderr = os.Stderr
	cmd.Stdin = bytes.NewReader(setupPayload)

	if err := cmd.Run(); err != nil {
		printRed("配置失败: " + err.Error())
	}

	// 立即显式销毁所有明文参数
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
		printRed("\n读取输入失败: " + err.Error())
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

func uninstallTGBot() {
	printYellow("\n确认卸载 TG Bot？所有配置将被删除。")
	fmt.Print("输入 y 确认卸载: ")
	var confirm string
	fmt.Scanln(&confirm)

	if confirm != "y" {
		printGreen("已取消卸载。")
		return
	}

	_ = runCmdSilent("systemctl", "stop", serviceName)
	_ = runCmdSilent("systemctl", "disable", serviceName)
	_ = os.Remove(serviceFile)
	_ = runCmdSilent("systemctl", "daemon-reload")
	_ = os.RemoveAll(installDir)

	printGreen("\n✅ TG Bot 已完全卸载。")
}

// ======================== 状态 ==============================

func showStatus() {
	printSkyBlue("\n--- TG Bot 状态 ---")

	// 二进制
	binPath := filepath.Join(installDir, binaryName)
	if _, err := os.Stat(binPath); err == nil {
		printGreen("二进制: 已安装")
	} else {
		printYellow("二进制: 未安装")
	}

	// 服务状态
	if err := runCmdSilent("systemctl", "is-active", "--quiet", serviceName); err == nil {
		printGreen("服务状态: 运行中 ✓")
	} else if runCmdSilent("systemctl", "is-enabled", "--quiet", serviceName) == nil {
		printYellow("服务状态: 已停止")
	} else {
		printYellow("服务状态: 未安装")
	}

	// 配置
	configPath := filepath.Join(installDir, "config.enc")
	if _, err := os.Stat(configPath); err == nil {
		printGreen("配置文件: 已初始化")
	} else {
		printYellow("配置文件: 未配置")
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
			fmt.Println("异常崩溃，内存已清理:", r)
			os.Exit(1)
		}
	}()

	disableCoreDumps()
	checkRoot()
	_ = checkArch()

	printBanner()
	showStatus()

	printYellow("1. 安装/更新 TG Bot")
	printYellow("2. 卸载 TG Bot")
	printYellow("0. 退出")

	fmt.Print("\n请选择: ")
	var choice string
	fmt.Scanln(&choice)

	switch choice {
	case "1":
		installTGBot()
	case "2":
		uninstallTGBot()
	case "0":
		os.Exit(0)
	default:
		printRed("无效选项")
		os.Exit(1)
	}
}
