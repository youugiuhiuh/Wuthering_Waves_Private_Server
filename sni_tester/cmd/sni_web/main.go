package main

import (
	"log"
	"net/http"
	"os"

	"sni_tester/pkg"
	"sni_tester/webserver"
)

func main() {
	outputDir := "/data/local/tmp/sni_output"
	if d := os.Getenv("SNI_OUTPUT_DIR"); d != "" {
		outputDir = d
	}

	cfg, err := newConfig(outputDir)
	if err != nil {
		log.Fatalf("Failed to initialize network: %v", err)
	}

	if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, ""); err != nil {
		log.Printf("Warning: GeoDB download failed: %v", err)
	}

	engine, err := pkg.NewEngine(cfg)
	if err != nil {
		log.Fatalf("Failed to init engine: %v", err)
	}
	defer engine.Close()

	srv := webserver.NewServer(engine, cfg)
	mux := webserver.NewMux(srv)

	log.Println("SNI API: http://0.0.0.0:18080")
	log.Fatal(http.ListenAndServe("0.0.0.0:18080", webserver.Cors(mux)))
}

func newConfig(outputDir string) (pkg.Config, error) {
	cfg := pkg.DefaultConfig()
	cfg.OutputDir = outputDir
	cfg.Debug = true

	network, err := pkg.NewNetwork(false)
	if err != nil {
		return pkg.Config{}, err
	}
	cfg.Network = network
	return cfg, nil
}
