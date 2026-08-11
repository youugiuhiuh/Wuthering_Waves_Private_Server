package main

import (
	"encoding/csv"
	"errors"
	"fmt"
	"io"
	"math"
	"net"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

const blockBegin = "# BEGIN aegis-cf-preferred-ip"
const blockEnd = "# END aegis-cf-preferred-ip"

type Candidate struct {
	IP          string
	Loss, Speed float64
}
type HostMapping struct{ IP, Domain string }

type commandDeps struct {
	hostsPath, executablePath string
	runCFST                   func(binary, output string) error
}

func main() {
	continueRun, err := ensureElevated(os.Args[1:])
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(elevatedExitStatus(err))
	}
	if !continueRun {
		return
	}
	executablePath, err := os.Executable()
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
	if err := run(os.Args[1:], commandDeps{hostsPath: hostsPath, executablePath: executablePath, runCFST: runCFST}); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}

func elevatedExitStatus(err error) int {
	var exitError *exec.ExitError
	if errors.As(err, &exitError) {
		return exitError.ExitCode()
	}
	return 1
}

func run(args []string, deps commandDeps) error {
	if len(args) == 1 && args[0] == "--restore" {
		return mutateHosts(deps.hostsPath, func(content string) (string, error) {
			next, _, err := removeOwnedBlock(content)
			return next, err
		})
	}
	if len(args) == 0 {
		return errors.New("usage: aegis-cf-preferred-ip [--restore] domain")
	}
	for _, domain := range args {
		if !validDomain(domain) {
			return fmt.Errorf("invalid domain %q", domain)
		}
	}

	temp, err := os.MkdirTemp("", "aegis-cf-")
	if err != nil {
		return err
	}
	defer os.RemoveAll(temp)
	output := filepath.Join(temp, "result.csv")
	if err := deps.runCFST(filepath.Join(filepath.Dir(deps.executablePath), cfstName), output); err != nil {
		return err
	}
	file, err := os.Open(output)
	if err != nil {
		return err
	}
	defer file.Close()
	candidates, err := parseCandidates(file)
	if err != nil {
		return err
	}
	mappings := assignCandidates(candidates, args)
	return mutateHosts(deps.hostsPath, func(content string) (string, error) {
		return replaceOwnedBlock(content, mappings)
	})
}

func runCFST(binary, output string) error {
	dir := filepath.Dir(output)
	stdout, err := os.Create(filepath.Join(dir, "stdout"))
	if err != nil {
		return err
	}
	defer stdout.Close()
	stderr, err := os.Create(filepath.Join(dir, "stderr"))
	if err != nil {
		return err
	}
	defer stderr.Close()
	cmd := exec.Command(binary, "-o", output)
	cmd.Dir = filepath.Dir(binary)
	cmd.Stdout = stdout
	cmd.Stderr = stderr
	return cmd.Run()
}

func mutateHosts(path string, transform func(string) (string, error)) error {
	old, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	next, err := transform(string(old))
	if err != nil {
		return err
	}
	temp, err := os.CreateTemp(filepath.Dir(path), ".aegis-cf-hosts-")
	if err != nil {
		return err
	}
	name := temp.Name()
	defer os.Remove(name)
	if _, err = temp.WriteString(next); err == nil {
		err = temp.Chmod(0644)
	}
	if closeErr := temp.Close(); err == nil {
		err = closeErr
	}
	if err != nil {
		return err
	}
	return os.Rename(name, path)
}

func parseCandidates(r io.Reader) ([]Candidate, error) {
	records, err := csv.NewReader(r).ReadAll()
	if err != nil {
		return nil, err
	}

	var candidates []Candidate
	for _, row := range records[1:] {
		if len(row) < 6 {
			continue
		}
		ip := strings.TrimSpace(row[0])
		if strings.Contains(ip, ":") || net.ParseIP(ip).To4() == nil {
			continue
		}
		loss, lossErr := strconv.ParseFloat(strings.TrimSuffix(strings.TrimSpace(row[3]), "%"), 64)
		fields := strings.Fields(row[5])
		if len(fields) == 0 {
			continue
		}
		speed, speedErr := strconv.ParseFloat(fields[0], 64)
		if lossErr == nil && speedErr == nil && !math.IsNaN(loss) && !math.IsInf(loss, 0) && !math.IsNaN(speed) && !math.IsInf(speed, 0) {
			candidates = append(candidates, Candidate{IP: ip, Loss: loss, Speed: speed})
		}
	}
	if len(candidates) == 0 {
		return nil, errors.New("CloudflareSpeedTest produced no valid IPv4 candidates")
	}
	sort.SliceStable(candidates, func(i, j int) bool {
		if candidates[i].Loss != candidates[j].Loss {
			return candidates[i].Loss < candidates[j].Loss
		}
		return candidates[i].Speed > candidates[j].Speed
	})
	return candidates, nil
}

func assignCandidates(candidates []Candidate, domains []string) []HostMapping {
	mappings := make([]HostMapping, len(domains))
	for i, domain := range domains {
		mappings[i] = HostMapping{IP: candidates[i%len(candidates)].IP, Domain: domain}
	}
	return mappings
}

func validDomain(domain string) bool {
	domain = strings.TrimSuffix(strings.TrimSpace(domain), ".")
	if domain == "" || len(domain) > 253 || net.ParseIP(domain) != nil {
		return false
	}
	for _, label := range strings.Split(domain, ".") {
		if len(label) == 0 || len(label) > 63 || label[0] == '-' || label[len(label)-1] == '-' {
			return false
		}
		for _, r := range label {
			if !((r >= 'a' && r <= 'z') || (r >= 'A' && r <= 'Z') || (r >= '0' && r <= '9') || r == '-') {
				return false
			}
		}
	}
	return true
}

func removeOwnedBlock(content string) (string, bool, error) {
	start, end, found, err := ownedBlockRange(content)
	if err != nil {
		return "", false, err
	}
	if !found {
		return content, false, nil
	}
	return content[:start] + content[end:], true, nil
}

func ownedBlockRange(content string) (start, end int, found bool, err error) {
	start, end = -1, -1
	offset := 0
	inBlock := false
	for _, line := range strings.SplitAfter(content, "\n") {
		marker := strings.TrimSuffix(strings.TrimSuffix(line, "\n"), "\r")
		switch marker {
		case blockBegin:
			if inBlock || found {
				return 0, 0, false, errors.New("existing preferred-IP markers contain duplicate blocks")
			}
			inBlock = true
			start = offset
		case blockEnd:
			if !inBlock {
				return 0, 0, false, errors.New("existing preferred-IP block has an orphan end marker")
			}
			inBlock = false
			found = true
			end = offset + len(line)
		}
		offset += len(line)
	}
	if inBlock {
		return 0, 0, false, errors.New("existing preferred-IP block has no end marker")
	}
	return start, end, found, nil
}

func replaceOwnedBlock(content string, mappings []HostMapping) (string, error) {
	remaining, _, err := removeOwnedBlock(content)
	if err != nil {
		return "", err
	}
	newline := "\n"
	if strings.Contains(content, "\r\n") {
		newline = "\r\n"
	}
	var block strings.Builder
	block.WriteString(blockBegin + newline)
	for _, mapping := range mappings {
		block.WriteString(mapping.IP + " " + mapping.Domain + newline)
	}
	block.WriteString(blockEnd + newline)
	if remaining != "" && !strings.HasSuffix(remaining, "\n") {
		remaining += newline
	}
	return remaining + block.String(), nil
}
