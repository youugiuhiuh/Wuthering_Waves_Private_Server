package handler

import (
	"encoding/base64"
	"log"
	"net/http"
	"strings"
	"time"

	"github.com/go-chi/chi/v5"
	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/cache"
	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/config"
	"github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/format"
	grpcclient "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/grpc"
)

var (
	grpcClient *grpcclient.Client
	lruCache   *cache.LRU
)

func Init(cfg *config.Config) error {
	var err error
	grpcClient, err = grpcclient.New(cfg.AegisGrpc)
	if err != nil {
		return err
	}
	lruCache = cache.NewWithTTL(1024, time.Duration(cfg.CacheTTL)*time.Second)
	return nil
}

func SubscriptionHandler(cfg *config.Config) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		token := chi.URLParam(r, "token")
		if token == "" {
			http.Error(w, "missing token", http.StatusBadRequest)
			return
		}
		ua := r.UserAgent()
		explicitFormat := r.URL.Query().Get("format")
		formatType := detectFormat(ua, explicitFormat)

		setSubscriptionHeaders(w, token)
		cacheKey := token + ":" + formatType
		if cached, ok := lruCache.Get(cacheKey); ok {
			writeResponse(w, formatType, cached.(string))
			return
		}

		configs, err := grpcClient.GetConfigs(token)
		if err != nil {
			log.Printf("gRPC error for token %s***: %v", safePrefix(token), err)
			http.Error(w, "internal error", http.StatusInternalServerError)
			return
		}

		var output string
		switch formatType {
		case "clash":
			output, err = format.ToClashYAML(configs)
		case "singbox":
			output, err = format.ToSingBox(configs)
		case "xray":
			output, err = format.ToXrayJSON(configs)
		case "base64":
			output = format.ToBase64List(configs)
		case "html":
			renderHTML(w, configs)
			return
		default:
			output = format.ToURIPlain(configs)
		}
		if err != nil {
			log.Printf("format error: %v", err)
			http.Error(w, "format error", http.StatusInternalServerError)
			return
		}

		lruCache.Set(cacheKey, output)
		writeResponse(w, formatType, output)
	}
}

func QRHandler(cfg *config.Config) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		token := chi.URLParam(r, "token")
		if token == "" {
			http.Error(w, "missing token", http.StatusBadRequest)
			return
		}
		http.Redirect(w, r, "/sub/"+token+"?format=base64", http.StatusFound)
	}
}

func detectFormat(ua, explicit string) string {
	if explicit != "" {
		return explicit
	}
	uaLower := strings.ToLower(ua)
	clash := []string{"clash", "stash", "surge", "clash-verge", "clash_verge", "cfw"}
	for _, kw := range clash {
		if strings.Contains(uaLower, kw) {
			return "clash"
		}
	}
	singbox := []string{"sing-box", "singbox", "hiddify", "karing"}
	for _, kw := range singbox {
		if strings.Contains(uaLower, kw) {
			return "singbox"
		}
	}
	xray := []string{"xray", "x-ui", "3x-ui", "nekobox"}
	for _, kw := range xray {
		if strings.Contains(uaLower, kw) {
			return "xray"
		}
	}
	base64 := []string{"shadowrocket", "v2rayng", "v2rayn", "nekoray", "v2ray", "v2fly", "fair", "pharos"}
	for _, kw := range base64 {
		if strings.Contains(uaLower, kw) {
			return "base64"
		}
	}
	if strings.Contains(uaLower, "mozilla") {
		return "html"
	}
	return "uri"
}

func writeResponse(w http.ResponseWriter, formatType, content string) {
	var ct string
	switch formatType {
	case "clash":
		ct = "text/yaml; charset=utf-8"
	case "singbox":
		ct = "application/json; charset=utf-8"
	case "xray":
		ct = "text/plain; charset=utf-8"
	default:
		ct = "text/plain; charset=utf-8"
	}
	w.Header().Set("Content-Type", ct)
	w.Header().Set("Cache-Control", "no-store")
	w.Write([]byte(content))
}

func setSubscriptionHeaders(w http.ResponseWriter, token string) {
	w.Header().Set("Subscription-Userinfo", "upload=0; download=0; total=1099511627776; expire=0")
	w.Header().Set("Profile-Update-Interval", "12")
	w.Header().Set("Profile-Title", base64.StdEncoding.EncodeToString([]byte("WWPS Subscription")))
	w.Header().Set("Support-Url", "https://t.me/wwps_support")
	w.Header().Set("Profile-Web-Page-Url", "/sub/"+token)
}

func safePrefix(token string) string {
	if len(token) > 4 {
		return token[:4]
	}
	return token
}
