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
	snipb "go_engine/proto"
	"google.golang.org/protobuf/proto"
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

func DedupeSorted(list []string) []string {
	if len(list) == 0 {
		return []string{}
	}
	sort.Strings(list)
	res := []string{}
	for i, d := range list {
		if d == "" {
			continue
		}
		if i == 0 || d != list[i-1] {
			res = append(res, d)
		}
	}
	return res
}

func WriteProtobufDualDomainFile(pqDomains, classicDomains []string, filePath string) error {
	uniquePQ := DedupeSorted(pqDomains)
	uniqueClassic := DedupeSorted(classicDomains)

	if len(uniquePQ) == 0 && len(uniqueClassic) == 0 {
		return nil
	}

	pb := &snipb.DomainList{
		PqDomains:      uniquePQ,
		ClassicDomains: uniqueClassic,
	}

	data, err := proto.Marshal(pb)
	if err != nil {
		return fmt.Errorf("failed to marshal protobuf: %w", err)
	}

	if err := os.WriteFile(filePath, data, 0644); err != nil {
		return fmt.Errorf("failed to write file: %w", err)
	}

	return nil
}

func ParseProtobufDomainList(data []byte) (*snipb.DomainList, error) {
	var pb snipb.DomainList
	if err := proto.Unmarshal(data, &pb); err != nil {
		return nil, fmt.Errorf("failed to unmarshal protobuf: %w", err)
	}
	return &pb, nil
}

func WriteProtobufDomainFile(domains []string, filePath string) error {
	return WriteProtobufDualDomainFile(nil, domains, filePath)
}

func ParseProtobufDomains(data []byte) ([]string, error) {
	pb, err := ParseProtobufDomainList(data)
	if err != nil {
		return nil, err
	}

	all := append([]string{}, pb.PqDomains...)
	all = append(all, pb.ClassicDomains...)
	return DedupeSorted(all), nil
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
	return SaveBatchDual(targetDir, nil, m, db)
}

func SaveBatchDual(targetDir string, pqMap, classicMap map[string][]string, db *badger.DB) error {
	allCountries := make(map[string]struct{})
	for c := range pqMap {
		allCountries[c] = struct{}{}
	}
	for c := range classicMap {
		allCountries[c] = struct{}{}
	}

	for country := range allCountries {
		filename := fmt.Sprintf("%s.pb", strings.ToUpper(country))
		targetPath := filepath.Join(targetDir, filename)
		_ = os.MkdirAll(targetDir, 0o755)

		var existPQ []string
		var existClassic []string
		if data, err := os.ReadFile(targetPath); err == nil {
			if dl, err := ParseProtobufDomainList(data); err == nil {
				existPQ = dl.PqDomains
				existClassic = dl.ClassicDomains
			}
		}

		mergedPQ := append(existPQ, pqMap[country]...)
		mergedClassic := append(existClassic, classicMap[country]...)

		if len(mergedPQ) > 0 || len(mergedClassic) > 0 {
			_ = WriteProtobufDualDomainFile(mergedPQ, mergedClassic, targetPath)
		}
	}

	if db != nil && len(allCountries) > 0 {
		now := time.Now().Unix()
		ttl := time.Duration(30) * 24 * time.Hour

		wb := db.NewWriteBatch()
		defer wb.Cancel()

		for country := range allCountries {
			allList := append([]string{}, pqMap[country]...)
			allList = append(allList, classicMap[country]...)
			for _, domain := range allList {
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
		_ = wb.Flush()
	}
	return nil
}
