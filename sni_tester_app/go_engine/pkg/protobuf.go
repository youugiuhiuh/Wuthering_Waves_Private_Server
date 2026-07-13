package pkg

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/dgraph-io/badger/v4"
	"google.golang.org/protobuf/proto"
	snipb "go_engine/proto"
)

func keyPrefixSuccess() []byte {
	return []byte("success:")
}

func strKeyPrefixSuccess() string {
	return "success:"
}

func isNumeric(s string) bool {
	if len(s) == 0 {
		return false
	}
	for i := 0; i < len(s); i++ {
		if s[i] < '0' || s[i] > '9' {
			return false
		}
	}
	return true
}

func WriteProtobufDomainFile(domains []string, filePath string) error {
	if len(domains) == 0 {
		return nil
	}

	sort.Strings(domains)
	uniqueDomains := []string{}
	for i, d := range domains {
		if i == 0 || d != domains[i-1] {
			uniqueDomains = append(uniqueDomains, d)
		}
	}

	pb := &snipb.DomainList{Domains: uniqueDomains}

	data, err := proto.Marshal(pb)
	if err != nil {
		return fmt.Errorf("failed to marshal protobuf: %w", err)
	}

	if err := os.WriteFile(filePath, data, 0644); err != nil {
		return fmt.Errorf("failed to write file: %w", err)
	}

	return nil
}

func ParseProtobufDomains(data []byte) ([]string, error) {
	var pb snipb.DomainList
	if err := proto.Unmarshal(data, &pb); err != nil {
		return nil, fmt.Errorf("failed to unmarshal protobuf: %w", err)
	}

	domains := []string{}
	for _, domain := range pb.Domains {
		if domain != "" && strings.Contains(domain, ".") {
			domains = append(domains, domain)
		}
	}

	return domains, nil
}

func CleanDomain(raw string) string {
	raw = strings.TrimSpace(raw)
	if len(raw) == 0 || raw[0] == '#' || (len(raw) >= 2 && raw[0:2] == "//") {
		return ""
	}

	var parts []string
	if idx := strings.IndexByte(raw, ','); idx != -1 {
		parts = strings.SplitN(raw, ",", 3)
	} else if strings.IndexByte(raw, '\t') != -1 {
		parts = strings.Split(raw, "\t")
	} else {
		parts = strings.Fields(raw)
	}

	for _, part := range parts {
		part = strings.TrimSpace(part)
		if len(part) == 0 {
			continue
		}
		if isNumeric(part) {
			continue
		}
		if strings.IndexByte(part, '.') != -1 {
			part = strings.Trim(part, `"',`)
			if idx := strings.IndexByte(part, ':'); idx != -1 {
				part = part[:idx]
			}
			if len(part) <= 2 && (part == "A" || part == "B" || part == "ID") {
				continue
			}
			return part
		}
	}
	return ""
}

func LoadExistingBinFiles(dir string, m map[string]struct{}) {
	files, _ := filepath.Glob(filepath.Join(dir, "*.pb"))
	for _, f := range files {
		baseName := strings.ToUpper(filepath.Base(f))
		if baseName == "CN.PB" || baseName == "HK.PB" || baseName == "MO.PB" {
			continue
		}
		data, err := os.ReadFile(f)
		if err != nil {
			continue
		}
		domains, _ := ParseProtobufDomains(data)
		for _, domain := range domains {
			m[domain] = struct{}{}
		}
	}
}

func LoadExistingIntoMap(dir string, m map[string]struct{}) {
	files, _ := filepath.Glob(filepath.Join(dir, "*.txt"))
	for _, f := range files {
		baseName := strings.ToUpper(filepath.Base(f))
		if baseName == "CN.TXT" || baseName == "HK.TXT" || baseName == "MO.TXT" {
			continue
		}
		file, err := os.Open(f)
		if err != nil {
			continue
		}
		sc := bufio.NewScanner(file)
		for sc.Scan() {
			d := CleanDomain(sc.Text())
			if d != "" {
				m[d] = struct{}{}
			}
		}
		file.Close()
	}
}

func DedupeStrings(sorted []string) []string {
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

func SaveBatch(targetDir string, m map[string][]string, db *badger.DB) error {
	for country, list := range m {
		filename := fmt.Sprintf("%s.pb", strings.ToUpper(country))
		targetPath := filepath.Join(targetDir, filename)
		os.MkdirAll(targetDir, 0o755)

		existingDomains := []string{}
		if data, err := os.ReadFile(targetPath); err == nil {
			existingDomains, _ = ParseProtobufDomains(data)
		}

		allDomains := append(existingDomains, list...)
		if len(allDomains) > 0 {
			WriteProtobufDomainFile(allDomains, targetPath)
		}
	}

	if db != nil && len(m) > 0 {
		now := time.Now().Unix()
		ttl := time.Duration(30) * 24 * time.Hour

		wb := db.NewWriteBatch()
		defer wb.Cancel()

		for country, list := range m {
			for _, domain := range list {
				info := SuccessInfo{
					Domain:   domain,
					Country:  country,
					TestedAt: now,
				}
				data, _ := json.Marshal(info)
				key := append(keyPrefixSuccess(), domain...)
				wb.SetEntry(&badger.Entry{
					Key:       key,
					Value:     data,
					ExpiresAt: uint64(now) + uint64(ttl.Seconds()),
				})
			}
		}
		wb.Flush()
	}
	return nil
}
