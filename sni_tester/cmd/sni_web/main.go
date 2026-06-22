package main

import (
	"embed"
	"io/fs"
	"log"
	"net/http"
	"os"
	"sni_tester/pkg"
)

//go:embed static/*
var staticFiles embed.FS

func main() {
	outputDir := "/data/local/tmp/sni_output"
	if d := os.Getenv("SNI_OUTPUT_DIR"); d != "" {
		outputDir = d
	}

	cfg := pkg.DefaultConfig()
	cfg.OutputDir = outputDir
	cfg.Debug = true // skip network isolation on phone

	engine, err := pkg.NewEngine(cfg)
	if err != nil {
		log.Fatalf("Failed to init engine: %v", err)
	}
	defer engine.Close()

	srv := &Server{
		engine:      engine,
		cfg:         cfg,
		subscribers: make(map[chan pkg.ProgressEvent]struct{}),
	}

	mux := http.NewServeMux()
	mux.HandleFunc("GET /", func(w http.ResponseWriter, r *http.Request) {
		sub, _ := fs.Sub(staticFiles, "static")
		http.FileServer(http.FS(sub)).ServeHTTP(w, r)
	})
	mux.HandleFunc("GET /api/progress", srv.handleSSE)
	mux.HandleFunc("POST /api/start", srv.handleStart)
	mux.HandleFunc("POST /api/stop", srv.handleStop)
	mux.HandleFunc("GET /api/status", srv.handleStatus)
	mux.HandleFunc("GET /api/download", srv.handleDownload)
	mux.HandleFunc("POST /api/upload", srv.handleUpload)

	log.Println("SNI Web UI: http://localhost:8080")
	log.Fatal(http.ListenAndServe(":8080", mux))
}
