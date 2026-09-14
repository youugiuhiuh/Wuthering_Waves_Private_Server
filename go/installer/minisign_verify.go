package main

import (
	"fmt"
	"os"
	"strings"
	"time"

	"aead.dev/minisign"

	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/go/installer/i18n"
)

type MinisigInfo struct {
	TrustedComment string
}

type minisignKeyEntry struct {
	PublicKey string
	ExpiresAt string // YYYY-MM-DD, empty = expired (key without date is invalid)
	RetiredAt string // YYYY-MM-DD, empty = 仍活跃
}

// isActive 表示该密钥仍用于验证新版本（尚未退役为历史密钥）。
// 注意：活跃 ≠ 未过期；过期判断由 expired() 单独负责。
func (e *minisignKeyEntry) isActive() bool {
	return e.RetiredAt == ""
}

func (e *minisignKeyEntry) expired() bool {
	if e.ExpiresAt == "" {
		return true
	}
	if _, err := time.Parse("2006-01-02", e.ExpiresAt); err != nil {
		return true
	}
	// 语义必须与 Rust 侧 key_expired 一致：按【日期字符串严格比较】，
	// 即 expires_at == 今天 → 未过期（仍可用）。
	// 历史实现用 time.Now().After(当日 00:00Z)，当天即视为过期，
	// 与 Rust 相差约 24h（F5-1）。空值/畸形日期一律过期（fail-closed）。
	return time.Now().UTC().Format("2006-01-02") > e.ExpiresAt
}

// 活跃密钥：验证新版本，会检查过期。
var minisignActiveKeys = []minisignKeyEntry{
	{PublicKey: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf", ExpiresAt: "2027-07-02", RetiredAt: ""},
}

// 历史密钥：仅验证用旧钥签的历史版本，刻意不检查过期。
var minisignHistoricalKeys = []minisignKeyEntry{}

// verifyMinisign 验证签名。先试活跃密钥（检查过期），再试历史密钥（忽略过期）。
func verifyMinisign(binaryPath, sigPath string, active, historical []minisignKeyEntry) (*MinisigInfo, error) {
	binaryData, err := os.ReadFile(binaryPath)
	if err != nil {
		return nil, fmt.Errorf("读取二进制文件失败: %w", err)
	}

	sigBytes, err := os.ReadFile(sigPath)
	if err != nil {
		return nil, fmt.Errorf("读取签名文件失败: %w", err)
	}

	// 活跃密钥：跳过已退役与已过期者
	for _, entry := range active {
		if !entry.isActive() || entry.expired() {
			continue
		}
		if info := tryVerify(entry, binaryData, sigBytes); info != nil {
			return info, nil
		}
	}

	// 历史密钥：不检查过期（否则用旧钥签的历史版本永远无法验证）
	for _, entry := range historical {
		if info := tryVerify(entry, binaryData, sigBytes); info != nil {
			return info, nil
		}
	}

	return nil, fmt.Errorf("minisign 验证失败: 无匹配公钥")
}

// tryVerify 用单把密钥尝试验证。失败返回 nil（不区分原因，
// 由调用方继续尝试下一把钥）。
func tryVerify(entry minisignKeyEntry, binaryData, sigBytes []byte) *MinisigInfo {
	var pubKey minisign.PublicKey
	if err := pubKey.UnmarshalText([]byte(entry.PublicKey)); err != nil {
		return nil
	}
	if !minisign.Verify(pubKey, binaryData, sigBytes) {
		return nil
	}
	var sig minisign.Signature
	if err := sig.UnmarshalText(sigBytes); err != nil {
		return nil
	}
	return &MinisigInfo{TrustedComment: sig.TrustedComment}
}

func parseTrustedComment(comment string) (version string, assetName string, err error) {
	parts := strings.SplitN(comment, ":", 2)
	if len(parts) != 2 {
		return "", "", fmt.Errorf("无效的可信注释格式: %s", comment)
	}
	return parts[0], parts[1], nil
}

// requireMinisign 是签名硬校验的唯一判定点。
//
// 签名缺失或验证失败 → 返回错误，调用方必须拒绝安装。
// 若这里改成「仅告警后继续」，攻击者只需删除 .minisig 资产即可
// 完全绕过签名验证（这正是本函数存在的理由）。
//
// 抽成纯函数是为了让这处安全关键判定成为可测单元：它原先内联在
// downloadAndDeployAegis 中，无注入接缝。实测：去掉 return 改回
// printYellow 后，整个测试套件仍然全绿（即无任何回归保护）。
func requireMinisign(passed bool) error {
	if !passed {
		return fmt.Errorf("%s", i18n.T("minisign.missing_fatal"))
	}
	return nil
}

// matchTrustedComment 校验签名 trusted comment 的版本与资产名。
//
// 纯函数，便于直接单测（原先这两段判断内联在 main.go 的
// downloadAndDeployAegis 里，依赖全局与网络，无注入接缝，
// 导致安全关键逻辑零测试覆盖 —— 注入 HasPrefix 回归时全测试仍绿）。
//
// 版本必须**精确相等**：HasPrefix / contains 会放行 "v1.5.3-evil"、"xv1.5.3"。
func matchTrustedComment(gotVersion, gotAsset, expectedVersion, expectedAsset string) error {
	if gotVersion != expectedVersion {
		return fmt.Errorf("%s", i18n.T("minisign.version_mismatch", expectedVersion, gotVersion))
	}
	if gotAsset != expectedAsset {
		return fmt.Errorf("%s", i18n.T("minisign.asset_mismatch", expectedAsset, gotAsset))
	}
	return nil
}

func findMinisigAsset(release *latestRelease, binaryName string) *releaseAsset {
	sigName := binaryName + ".minisig"
	for i := range release.Assets {
		if release.Assets[i].Name == sigName {
			return &release.Assets[i]
		}
	}
	return nil
}
