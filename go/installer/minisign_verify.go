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

var minisignPublicKeys = []minisignKeyEntry{
	{PublicKey: "RWS6qEwIdsvM7UppXGmoZ+nksGYr+sc6POwW2Tdby1mZhpfiipMAu7ts", ExpiresAt: "2027-07-02"},
	{PublicKey: "RWRtqaFpUXIpMym7ZGrOAO/4VuP6vV08QZKODsB/I4Mav/WOgi5VTwPS", ExpiresAt: "2027-07-02"},
}

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
