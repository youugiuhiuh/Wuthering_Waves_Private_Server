package main

import (
	"encoding/json"
	"log"
	"net/http"
	"os"
	"time"

	"sni_tester/pkg"
)

var startTime = time.Now()

func cors(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Access-Control-Allow-Origin", "*")
		w.Header().Set("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
		w.Header().Set("Access-Control-Allow-Headers", "Content-Type")
		if r.Method == "OPTIONS" {
			w.WriteHeader(204)
			return
		}
		next.ServeHTTP(w, r)
	})
}

func main() {
	outputDir := "/data/local/tmp/sni_output"
	if d := os.Getenv("SNI_OUTPUT_DIR"); d != "" {
		outputDir = d
	}

	cfg := pkg.DefaultConfig()
	cfg.OutputDir = outputDir
	cfg.Debug = true

	if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, ""); err != nil {
		log.Printf("Warning: GeoDB download failed (will still try): %v", err)
	}

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
	mux.HandleFunc("GET /api/health", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]interface{}{
			"status":  "ok",
			"uptime":  time.Since(startTime).String(),
			"version": "1.0",
		})
	})
	mux.HandleFunc("GET /api/progress", srv.handleSSE)
	mux.HandleFunc("POST /api/start", srv.handleStart)
	mux.HandleFunc("POST /api/stop", srv.handleStop)
	mux.HandleFunc("GET /api/status", srv.handleStatus)
	mux.HandleFunc("GET /api/download", srv.handleDownload)
	mux.HandleFunc("POST /api/upload", srv.handleUpload)
	mux.HandleFunc("GET /api/files", srv.HandleListFiles)
	mux.HandleFunc("DELETE /api/files", srv.HandleDeleteFile)

	log.Println("SNI API: http://0.0.0.0:18080")
	log.Fatal(http.ListenAndServe("0.0.0.0:18080", cors(mux)))
}
