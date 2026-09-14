package main

import (
	"fmt"
	"os"
	"strings"
	"time"

	"aead.dev/minisign"
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
	t, err := time.Parse("2006-01-02", e.ExpiresAt)
	if err != nil {
		return true
	}
	return time.Now().After(t)
}

// 活跃密钥：验证新版本，会检查过期。
var minisignActiveKeys = []minisignKeyEntry{
	{PublicKey: "RWTZPf3UsUDo9hPmWcOp+0TcwRLWHmOkNCGPw3kXcM3x5awPEzR3Y3Sf", ExpiresAt: "2027-07-02", RetiredAt: ""},
}

// 历史密钥：仅验证用旧钥签的历史版本，刻意不检查过期。
var minisignHistoricalKeys = []minisignKeyEntry{}

// 过渡别名：迁移期保留，供尚未改用双表接口的调用点（main.go:873）继续编译。
// 语义等同于活跃表。
// TODO(Task 6): verifyMinisign 改为接收 active/historical 双表后删除本别名。
var minisignPublicKeys = minisignActiveKeys

func verifyMinisign(binaryPath, sigPath string, pubKeys []minisignKeyEntry) (*MinisigInfo, error) {
	binaryData, err := os.ReadFile(binaryPath)
	if err != nil {
		return nil, fmt.Errorf("读取二进制文件失败: %w", err)
	}

	sigBytes, err := os.ReadFile(sigPath)
	if err != nil {
		return nil, fmt.Errorf("读取签名文件失败: %w", err)
	}

	for _, entry := range pubKeys {
		if entry.expired() {
			continue
		}
		var pubKey minisign.PublicKey
		if err := pubKey.UnmarshalText([]byte(entry.PublicKey)); err != nil {
			continue
		}
		if minisign.Verify(pubKey, binaryData, sigBytes) {
			var sig minisign.Signature
			if err := sig.UnmarshalText(sigBytes); err != nil {
				continue
			}
			return &MinisigInfo{
				TrustedComment: sig.TrustedComment,
			}, nil
		}
	}

	return nil, fmt.Errorf("minisign 验证失败: 无匹配公钥")
}

func parseTrustedComment(comment string) (version string, assetName string, err error) {
	parts := strings.SplitN(comment, ":", 2)
	if len(parts) != 2 {
		return "", "", fmt.Errorf("无效的可信注释格式: %s", comment)
	}
	return parts[0], parts[1], nil
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
