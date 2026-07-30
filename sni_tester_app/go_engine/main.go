package main

/*
#include <stdlib.h>
*/
import "C"
import (
	"context"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"sync"
	"time"

	"go_engine/pkg"
	"go_engine/webserver"
)

var (
	globalEngine *pkg.Engine
	globalServer *http.Server
	globalMu     sync.Mutex
)

//export StartServer
func StartServer(cBaseDir *C.char, cOutputDir *C.char) C.int {
	baseDir := C.GoString(cBaseDir)
	outputDir := C.GoString(cOutputDir)

	globalMu.Lock()
	defer globalMu.Unlock()

	if globalServer != nil {
		return 0
	}

	cfg := pkg.DefaultConfig()
	cfg.OutputDir = outputDir
	cfg.Debug = true
	cfg.GeoDBFile = filepath.Join(baseDir, cfg.GeoDBFile)
	cfg.GeoASNFile = filepath.Join(baseDir, cfg.GeoASNFile)
	cfg.BadgerDBDir = filepath.Join(baseDir, cfg.BadgerDBDir)

	os.MkdirAll(outputDir, 0o755)

	if err := pkg.PrepareGeoDBs(cfg.GeoDBFile, cfg.GeoASNFile, ""); err != nil {
		log.Printf("Warning: GeoDB download failed: %v", err)
	}

	engine, err := pkg.NewEngine(cfg)
	if err != nil {
		log.Printf("Failed to init engine: %v", err)
		return -1
	}
	globalEngine = engine

	srv := webserver.NewServer(engine, cfg)
	mux := webserver.NewMux(srv)

	globalServer = &http.Server{
		Addr:    "0.0.0.0:18080",
		Handler: webserver.Cors(mux),
	}

	go func() {
		log.Println("SNI API: http://0.0.0.0:18080")
		if err := globalServer.ListenAndServe(); err != http.ErrServerClosed {
			log.Printf("Server error: %v", err)
		}
	}()

	return 0
}

//export StopServer
func StopServer() C.int {
	globalMu.Lock()
	defer globalMu.Unlock()

	if globalServer != nil {
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		defer cancel()
		globalServer.Shutdown(ctx)
		globalServer = nil
	}
	if globalEngine != nil {
		globalEngine.Close()
		globalEngine = nil
	}
	return 0
}

func main() {}
