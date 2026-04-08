package main

import (
	"encoding/binary"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Println("Usage: bin2pb <input_dir> [output_dir]")
		fmt.Println("  input_dir: directory containing reality/ and xhttp/ subdirectories with .bin files")
		fmt.Println("  output_dir: output directory for .pb files (default: input_dir)")
		fmt.Println("Example: bin2pb rust/tgbot/src/resources/sni")
		os.Exit(1)
	}

	inputDir := os.Args[1]
	outputDir := inputDir
	if len(os.Args) >= 3 {
		outputDir = os.Args[2]
	}

	// Create output directory if needed
	os.MkdirAll(outputDir, 0755)

	// Process reality and xhttp directories
	subdirs := []string{"reality", "xhttp"}
	totalFiles := 0
	totalDomains := 0
	allCountryCodes := make(map[string]bool)

	for _, subdir := range subdirs {
		inputPath := filepath.Join(inputDir, subdir)

		if _, err := os.Stat(inputPath); os.IsNotExist(err) {
			fmt.Printf("Skipping %s: directory not found\n", subdir)
			continue
		}

		files, err := filepath.Glob(filepath.Join(inputPath, "*.bin"))
		if err != nil {
			fmt.Printf("Error scanning %s: %v\n", inputPath, err)
			continue
		}

		fmt.Printf("\nProcessing %s (%d files):\n", subdir, len(files))

		for _, binFile := range files {
			countryCode := strings.TrimSuffix(filepath.Base(binFile), ".bin")
			pbFile := filepath.Join(outputDir, countryCode+".pb")

			// Check if already processed (from other subdir)
			if allCountryCodes[countryCode] {
				// Merge domains
				domains, err := readBinFile(binFile)
				if err != nil {
					fmt.Printf("  [ERROR] %s: %v\n", countryCode, err)
					continue
				}

				// Read existing
				existingDomains, _ := readProtobufFile(pbFile)
				merged := mergeDomains(existingDomains, domains)

				if err := writeProtobufFile(merged, pbFile); err != nil {
					fmt.Printf("  [ERROR] %s: %v\n", countryCode, err)
					continue
				}

				fmt.Printf("  [MERGE] %s: + %d domains =%d total\n", countryCode, len(domains), len(merged))
				totalDomains += len(domains)
				continue
			}

			domains, err := readBinFile(binFile)
			if err != nil {
				fmt.Printf("  [ERROR] %s: %v\n", countryCode, err)
				continue
			}

			if err := writeProtobufFile(domains, pbFile); err != nil {
				fmt.Printf("  [ERROR] %s: %v\n", countryCode, err)
				continue
			}

			fmt.Printf("  [OK] %s: %d domains\n", countryCode, len(domains))
			allCountryCodes[countryCode] = true
			totalFiles++
			totalDomains += len(domains)
		}
	}

	fmt.Printf("\n=== Summary ===\n")
	fmt.Printf("Total files converted: %d\n", totalFiles)
	fmt.Printf("Total domains: %d\n", totalDomains)

	// Verify output
	fmt.Printf("\n=== Verification ===\n")
	pbFiles, _ := filepath.Glob(filepath.Join(outputDir, "*.pb"))
	verifiedCount := 0
	verifiedDomains := 0
	for _, pbFile := range pbFiles {
		data, err := os.ReadFile(pbFile)
		if err != nil {
			fmt.Printf("[ERROR] %s: cannot read: %v\n", filepath.Base(pbFile), err)
			continue
		}

		domains, err := parseProtobuf(data)
		if err != nil {
			fmt.Printf("[ERROR] %s: cannot parse: %v\n", filepath.Base(pbFile), err)
			continue
		}

		// Verify domains are valid
		for _, d := range domains {
			if d == "" || !strings.Contains(d, ".") {
				fmt.Printf("[ERROR] %s: invalid domain: %s\n", filepath.Base(pbFile), d)
			}
		}

		fmt.Printf("[OK] %s: %d domains verified\n", filepath.Base(pbFile), len(domains))
		verifiedCount++
		verifiedDomains += len(domains)
	}

	fmt.Printf("\nVerified: %d files, %d domains\n", verifiedCount, verifiedDomains)

	if verifiedCount == totalFiles {
		fmt.Println("\nAll files converted successfully!")
	}
}

func readBinFile(filename string) ([]string, error) {
	data, err := os.ReadFile(filename)
	if err != nil {
		return nil, fmt.Errorf("cannot read file: %w", err)
	}

	var domains []string
	offset := 0

	for offset+2 <= len(data) {
		length := int(binary.BigEndian.Uint16(data[offset : offset+2]))
		offset += 2

		if length == 0 || length > 512 || offset+length > len(data) {
			break
		}

		domain := string(data[offset : offset+length])
		if domain != "" && strings.Contains(domain, ".") {
			domains = append(domains, domain)
		}
		offset += length
	}

	return domains, nil
}

func mergeDomains(existing, newDomains []string) []string {
	merged := append(existing, newDomains...)
	sort.Strings(merged)

	unique := make([]string, 0, len(merged))
	for i, d := range merged {
		if i == 0 || d != merged[i-1] {
			unique = append(unique, d)
		}
	}
	return unique
}

func readProtobufFile(filename string) ([]string, error) {
	data, err := os.ReadFile(filename)
	if err != nil {
		return nil, err
	}
	return parseProtobuf(data)
}

func parseProtobuf(data []byte) ([]string, error) {
	var domains []string
	offset := 0

	for offset < len(data) {
		// Read field tag
		if offset >= len(data) {
			break
		}
		tag := data[offset]
		offset++

		fieldNum := tag >> 3
		wireType := tag & 0x7

		if fieldNum != 1 || wireType != 2 {
			return nil, fmt.Errorf("unexpected field: num=%d, wire=%d", fieldNum, wireType)
		}

		// Read length
		length, n := readVarint(data[offset:])
		offset += n

		if offset+int(length) > len(data) {
			break
		}

		domain := string(data[offset : offset+int(length)])
		domains = append(domains, domain)
		offset += int(length)
	}

	return domains, nil
}

func writeProtobufFile(domains []string, filename string) error {
	// Sort and deduplicate
	sort.Strings(domains)
	uniqueDomains := []string{}
	for i, d := range domains {
		if i == 0 || d != domains[i-1] {
			uniqueDomains = append(uniqueDomains, d)
		}
	}

	var buf []byte

	for _, domain := range uniqueDomains {
		// Field tag: field number 1, wire type 2 (length-delimited)
		// tag = (field_number << 3) | wire_type = (1 << 3) | 2 = 10
		buf = append(buf, 10)

		// Length prefix
		buf = appendVarint(buf, uint64(len(domain)))

		// Domain string
		buf = append(buf, domain...)
	}

	if err := os.WriteFile(filename, buf, 0644); err != nil {
		return fmt.Errorf("cannot write file: %w", err)
	}

	return nil
}

func appendVarint(buf []byte, x uint64) []byte {
	for x >= 128 {
		buf = append(buf, byte(x&0x7f|0x80))
		x >>= 7
	}
	buf = append(buf, byte(x))
	return buf
}

func readVarint(data []byte) (uint64, int) {
	var x uint64
	var n int
	for i, b := range data {
		x |= uint64(b&0x7f) << (7 * i)
		n++
		if b&0x80 == 0 {
			break
		}
	}
	return x, n
}
