package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"strings"
	"sync"
	"sni_tester/pkg"
)

type Server struct {
	engine      *pkg.Engine
	cfg         pkg.Config
	cancel      func()
	mu          sync.Mutex
	running     bool
	subscribers map[chan pkg.ProgressEvent]struct{}
	results     *pkg.Result
	inputText   string
}

func (s *Server) handleUpload(w http.ResponseWriter, r *http.Request) {
	r.ParseMultipartForm(10 << 20)
	file, _, err := r.FormFile("file")
	if err != nil {
		http.Error(w, err.Error(), 400)
		return
	}
	defer file.Close()
	buf := new(bytes.Buffer)
	io.Copy(buf, file)
	s.inputText = buf.String()
	json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
}

func (s *Server) handleStart(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	if s.running {
		s.mu.Unlock()
		http.Error(w, "already running", 409)
		return
	}
	s.running = true
	s.mu.Unlock()

	var params struct {
		Workers int    `json:"workers"`
		DNS     string `json:"dns"`
		TTL     int    `json:"ttl"`
		Force   bool   `json:"force"`
		Reset   bool   `json:"reset"`
	}
	json.NewDecoder(r.Body).Decode(&params)

	cfg := s.cfg
	if params.Workers > 0 {
		cfg.FixedWorkers = params.Workers
	}
	if params.DNS != "" {
		cfg.DNSAddr = params.DNS
		cfg.UseBuiltinDNS = false
	}
	if params.TTL > 0 {
		cfg.TTLDays = params.TTL
	}
	cfg.ForceRetry = params.Force
	cfg.ResetAll = params.Reset

	ctx, cancel := context.WithCancel(context.Background())
	s.cancel = cancel

	var domains []string
	sc := bufio.NewScanner(strings.NewReader(s.inputText))
	for sc.Scan() {
		d := pkg.CleanDomain(sc.Text())
		if d != "" {
			domains = append(domains, d)
		}
	}

	go func() {
		result, err := s.engine.Run(ctx, domains, func(ev pkg.ProgressEvent) {
			s.broadcast(ev)
		})
		s.mu.Lock()
		s.running = false
		s.results = result
		s.mu.Unlock()
		s.broadcast(pkg.ProgressEvent{Type: "done", Progress: 1.0})
		if err != nil {
			log.Printf("Engine error: %v", err)
		}
	}()

	json.NewEncoder(w).Encode(map[string]string{"status": "started"})
}

func (s *Server) handleStop(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	if s.cancel != nil {
		s.cancel()
	}
	s.running = false
	s.mu.Unlock()
	json.NewEncoder(w).Encode(map[string]string{"status": "stopped"})
}

func (s *Server) handleStatus(w http.ResponseWriter, r *http.Request) {
	s.mu.Lock()
	running := s.running
	results := s.results
	s.mu.Unlock()

	resp := map[string]interface{}{"running": running}
	if results != nil {
		resp["stats"] = results.Stats
	}
	json.NewEncoder(w).Encode(resp)
}

func (s *Server) handleDownload(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "application/zip")
	w.Header().Set("Content-Disposition", "attachment; filename=sni_results.zip")

	zw := zip.NewWriter(w)
	filepath.Walk(s.cfg.OutputDir, func(path string, info os.FileInfo, err error) error {
		if err != nil || info.IsDir() || !strings.HasSuffix(info.Name(), ".pb") {
			return nil
		}
		f, _ := os.ReadFile(path)
		fh := &zip.FileHeader{Name: info.Name(), Method: zip.Deflate}
		out, _ := zw.CreateHeader(fh)
		out.Write(f)
		return nil
	})
	zw.Close()
}

func (s *Server) broadcast(event pkg.ProgressEvent) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for ch := range s.subscribers {
		select {
		case ch <- event:
		default:
		}
	}
}

func (s *Server) handleSSE(w http.ResponseWriter, r *http.Request) {
	flusher, ok := w.(http.Flusher)
	if !ok {
		http.Error(w, "Streaming unsupported", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "text/event-stream")
	w.Header().Set("Cache-Control", "no-cache")
	w.Header().Set("Connection", "keep-alive")
	ch := make(chan pkg.ProgressEvent, 100)
	s.mu.Lock()
	s.subscribers[ch] = struct{}{}
	s.mu.Unlock()

	notify := r.Context().Done()
	for {
		select {
		case <-notify:
			s.mu.Lock()
			delete(s.subscribers, ch)
			s.mu.Unlock()
			return
		case event := <-ch:
			data, _ := json.Marshal(event)
			fmt.Fprintf(w, "data: %s\n\n", data)
			flusher.Flush()
		}
	}
}
