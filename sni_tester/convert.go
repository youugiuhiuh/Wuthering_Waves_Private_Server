//go:build ignore

// SNI Binary Format Converter Tool
// Usage: go run convert.go [--verify]
//
// Converts .txt files to .bin format for faster parsing

package main

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

const (
	RealitySNI = "rust/tgbot/src/resources/sni/reality"
	XHTTPLSNI  = "rust/tgbot/src/resources/sni/xhttp"
)

func main() {
	if len(os.Args) > 1 && os.Args[1] == "--verify" {
		verifyConversion()
		return
	}

	fmt.Println("=== SNI Binary Format Converter ===")
	fmt.Println()

	dirs := []string{RealitySNI, XHTTPLSNI}

	for _, dir := range dirs {
		fmt.Printf("Processing: %s\n", dir)
		if err := convertDirectory(dir); err != nil {
			fmt.Printf("Error: %v\n", err)
		}
		fmt.Println()
	}

	fmt.Println("=== Conversion Complete ===")
	fmt.Println()
	fmt.Println("Run with --verify to validate the conversion")
}

func convertDirectory(dir string) error {
	entries, err := os.ReadDir(dir)
	if err != nil {
		return fmt.Errorf("failed to read directory: %w", err)
	}

	var txtFiles []string
	for _, entry := range entries {
		if !entry.IsDir() && strings.HasSuffix(entry.Name(), ".txt") {
			txtFiles = append(txtFiles, entry.Name())
		}
	}

	fmt.Printf("  Found %d .txt files\n", len(txtFiles))

	converted := 0
	errors := 0

	for _, txtFile := range txtFiles {
		txtPath := filepath.Join(dir, txtFile)
		binPath := filepath.Join(dir, strings.TrimSuffix(txtFile, ".txt")+".bin")

		count, err := convertFile(txtPath, binPath)
		if err != nil {
			fmt.Printf("  [ERROR] %s: %v\n", txtFile, err)
			errors++
			continue
		}

		fmt.Printf("  [OK] %s -> %s (%d domains)\n", txtFile, filepath.Base(binPath), count)
		converted++
	}

	fmt.Printf("  Converted: %d, Errors: %d\n", converted, errors)
	return nil
}

func convertFile(txtPath, binPath string) (int, error) {
	data, err := os.ReadFile(txtPath)
	if err != nil {
		return 0, fmt.Errorf("failed to read: %w", err)
	}

	domains := parseDomains(string(data))
	if len(domains) == 0 {
		return 0, fmt.Errorf("no domains found")
	}

	sort.Strings(domains)
	domains = dedupe(domains)

	var buf bytes.Buffer
	for _, d := range domains {
		binary.Write(&buf, binary.BigEndian, uint16(len(d)))
		buf.WriteString(d)
	}

	if err := os.WriteFile(binPath, buf.Bytes(), 0644); err != nil {
		return 0, fmt.Errorf("failed to write: %w", err)
	}

	return len(domains), nil
}

func parseDomains(content string) []string {
	var domains []string
	lines := strings.Split(content, "\n")
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line == "" || strings.HasPrefix(line, "#") || strings.HasPrefix(line, "//") {
			continue
		}

		clean := strings.Trim(line, "\"'")
		clean = strings.TrimSuffix(clean, ",")
		if idx := strings.Index(clean, ":"); idx != -1 {
			clean = clean[:idx]
		}
		clean = strings.TrimSpace(clean)

		if clean != "" && strings.Contains(clean, ".") {
			domains = append(domains, clean)
		}
	}
	return domains
}

func dedupe(sorted []string) []string {
	if len(sorted) == 0 {
		return sorted
	}
	result := []string{sorted[0]}
	for i := 1; i < len(sorted); i++ {
		if sorted[i] != result[len(result)-1] {
			result = append(result, sorted[i])
		}
	}
	return result
}

func verifyConversion() {
	fmt.Println("=== Verification Mode ===")
	fmt.Println()

	dirs := []string{RealitySNI, XHTTPLSNI}
	totalBin := 0
	totalTxt := 0

	for _, dir := range dirs {
		fmt.Printf("Verifying: %s\n", dir)

		entries, err := os.ReadDir(dir)
		if err != nil {
			fmt.Printf("  [ERROR] Cannot read directory: %v\n", err)
			continue
		}

		binCount := 0
		txtCount := 0

		for _, entry := range entries {
			if entry.IsDir() {
				continue
			}
			name := entry.Name()
			if strings.HasSuffix(name, ".bin") {
				binCount++
				path := filepath.Join(dir, name)
				count, err := countBinaryDomains(path)
				if err != nil {
					fmt.Printf("  [ERROR] %s: %v\n", name, err)
				} else {
					fmt.Printf("  [OK] %s (%d domains)\n", name, count)
				}
			} else if strings.HasSuffix(name, ".txt") {
				txtCount++
			}
		}

		fmt.Printf("  .bin files: %d, .txt files: %d\n\n", binCount, txtCount)
		totalBin += binCount
		totalTxt += txtCount
	}

	fmt.Printf("Total: %d .bin files, %d .txt files\n", totalBin, totalTxt)

	if totalTxt > 0 {
		fmt.Println()
		fmt.Println("WARNING: .txt files still exist! Remove them after verification.")
	}
}

func countBinaryDomains(path string) (int, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return 0, err
	}

	count := 0
	offset := 0
	for offset < len(data) {
		if offset+2 > len(data) {
			break
		}
		length := binary.BigEndian.Uint16(data[offset : offset+2])
		offset += 2
		if offset+int(length) > len(data) {
			break
		}
		offset += int(length)
		count++
	}

	return count, nil
}

func ReadBinaryDomains(r io.Reader) ([]string, error) {
	var domains []string
	for {
		var length uint16
		if err := binary.Read(r, binary.BigEndian, &length); err != nil {
			if err == io.EOF {
				break
			}
			return nil, err
		}

		buf := make([]byte, length)
		if _, err := io.ReadFull(r, buf); err != nil {
			return nil, err
		}
		domains = append(domains, string(buf))
	}
	return domains, nil
}
