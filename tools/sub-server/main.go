package main

import (
	"fmt"
	"log"
	"net/http"
	"os"

	"github.com/go-chi/chi/v5"
	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/config"
	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/handler"
	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/middleware"
)

var version = "dev"

func main() {
	cfg := config.Parse()
	if err := handler.Init(cfg); err != nil {
		log.Fatalf("handler init: %v", err)
	}
	r := chi.NewRouter()
	r.Use(middleware.RateLimit(cfg.RateLimit))
	r.Get("/sub/{token}", handler.SubscriptionHandler(cfg))
	r.Get("/sub/{token}/qr", handler.QRHandler(cfg))
	r.Get("/health", func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/json")
		w.Write([]byte(`{"status":"ok","version":"` + version + `"}`))
	})
	addr := cfg.ListenAddr
	log.Printf("sub-server %s starting on %s", version, addr)
	if cfg.TLSCert != "" && cfg.TLSKey != "" {
		if err := http.ListenAndServeTLS(addr, cfg.TLSCert, cfg.TLSKey, r); err != nil {
			fmt.Fprintf(os.Stderr, "TLS server error: %v\n", err)
			os.Exit(1)
		}
	} else {
		if err := http.ListenAndServe(addr, r); err != nil {
			fmt.Fprintf(os.Stderr, "server error: %v\n", err)
			os.Exit(1)
		}
	}
}
