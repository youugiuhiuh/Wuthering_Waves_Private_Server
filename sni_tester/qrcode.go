package main

import (
	"bytes"
	"fmt"
	"strings"

	"github.com/skip2/go-qrcode"
)

type QRCode struct {
	Content string
}

func NewQRCode(content string) *QRCode {
	return &QRCode{Content: content}
}

func (q *QRCode) GenerateASCII() (string, error) {
	qr, err := qrcode.Encode(q.Content, qrcode.Medium, 256)
	if err != nil {
		return "", err
	}

	lines := strings.Split(string(qr), "\n")
	var asciiLines []string

	asciiLines = append(asciiLines, "┌"+strings.Repeat("─", len(lines[0]))+"┐")

	for i, line := range lines {
		if i == 0 || i == len(lines)-1 {
			continue
		}

		asciiLine := "│" + line + "│"
		asciiLines = append(asciiLines, asciiLine)
	}

	asciiLines = append(asciiLines, "└"+strings.Repeat("─", len(lines[0]))+"┘")

	return strings.Join(asciiLines, "\n"), nil
}

func (q *QRCode) GenerateTerminal() error {
	qr, err := qrcode.Encode(q.Content, qrcode.Medium, 256)
	if err != nil {
		return err
	}

	fmt.Println(qr)
	return nil
}

func (q *QRCode) GeneratePNG(filename string) error {
	err := qrcode.WriteFile(q.Content, qrcode.Medium, 256, filename+".png")
	return err
}

func GenerateConnectQRCode(ip string, port int) (*QRCode, error) {
	content := fmt.Sprintf("%s:%d", ip, port)
	return NewQRCode(content), nil
}

func DisplayWiFiADBQRCode(ip string, port int) error {
	qr, err := GenerateConnectQRCode(ip, port)
	if err != nil {
		return err
	}

	fmt.Println()
	fmt.Println(" ╔═══════════════════════════════════════════════════════════╗")
	fmt.Println(" ║           WiFi ADB 连接 - 请使用手机扫码                   ║")
	fmt.Println(" ╠═══════════════════════════════════════════════════════════╣")

	ascii, err := qr.GenerateASCII()
	if err != nil {
		return err
	}

	lines := strings.Split(ascii, "\n")
	for _, line := range lines {
		fmt.Printf(" ║  %s  ║\n", line)
	}

	fmt.Println(" ╠═══════════════════════════════════════════════════════════╣")
	fmt.Printf(" ║  连接地址: %s:%d                                    ║\n", ip, port)
	fmt.Println(" ║                                                           ║")
	fmt.Println(" ║  或者在手机上执行:                                        ║")
	fmt.Printf(" ║    adb connect %s:%d                                    ║\n", ip, port)
	fmt.Println(" ╚═══════════════════════════════════════════════════════════╝")
	fmt.Println()

	return nil
}

func DisplaySimpleQRCode(content string) error {
	qr := NewQRCode(content)

	ascii, err := qr.GenerateASCII()
	if err != nil {
		return err
	}

	fmt.Println()
	fmt.Println(" ┌─────────────────────────────────────┐")
	lines := strings.Split(ascii, "\n")
	for _, line := range lines {
		fmt.Printf(" │ %s │\n", line)
	}
	fmt.Println(" └─────────────────────────────────────┘")
	fmt.Printf("     连接地址: %s\n", content)
	fmt.Println()

	return nil
}

func IsQRCodeSupported() bool {
	return true
}

var _ bytes.Buffer
