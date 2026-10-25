package pkg

import (
	"context"
	"net"
	"os"
	"path/filepath"
	"testing"
	"time"
)

func startMockSurvivalServer(t *testing.T) (string, func()) {
	t.Helper()
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatalf("listen mock target: %v", err)
	}

	closed := make(chan struct{})
	go func() {
		for {
			conn, err := ln.Accept()
			if err != nil {
				select {
				case <-closed:
					return
				default:
					continue
				}
			}
			go func(c net.Conn) {
				defer c.Close()
				buf := make([]byte, 1024)
				c.SetDeadline(time.Now().Add(2 * time.Second))
				c.Read(buf)
			}(conn)
		}
	}()

	return ln.Addr().String(), func() {
		close(closed)
		ln.Close()
	}
}

func TestProbeDomainSurvival_Failure(t *testing.T) {
	ctx, cancel := context.WithTimeout(context.Background(), 1*time.Second)
	defer cancel()

	survived := ProbeDomainSurvival(ctx, "invalid.local", "192.0.2.1", 200*time.Millisecond)
	if survived {
		t.Errorf("expected unreachable IP to fail, got true")
	}
}

func TestRunProbeSurvival_E2E(t *testing.T) {
	tmpDir := t.TempDir()
	inFile := filepath.Join(tmpDir, "input.txt")
	outFile := filepath.Join(tmpDir, "survived.txt")

	content := "invalid-domain-that-will-not-resolve-1234567.org\n"
	if err := os.WriteFile(inFile, []byte(content), 0644); err != nil {
		t.Fatalf("write input: %v", err)
	}

	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()

	opts := SurvivalOptions{
		InputFile:  inFile,
		OutputFile: outFile,
		Workers:    2,
		Timeout:    2 * time.Second,
	}

	stats, err := RunProbeSurvival(ctx, opts)
	if err != nil {
		t.Fatalf("RunProbeSurvival failed: %v", err)
	}

	if stats.Total != 1 {
		t.Errorf("stats.Total = %d, want 1", stats.Total)
	}
	if stats.Dead != 1 {
		t.Errorf("stats.Dead = %d, want 1", stats.Dead)
	}

	if _, err := os.Stat(outFile); err != nil {
		t.Errorf("output file %s not created: %v", outFile, err)
	}
}
