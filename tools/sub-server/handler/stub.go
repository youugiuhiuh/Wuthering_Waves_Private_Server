package handler

import (
	"net/http"

	"github.com/NicholasDewar/Wuthering_Waves_Private_Server/tools/sub-server/config"
)

func Init(cfg *config.Config) error {
	return nil
}

func SubscriptionHandler(cfg *config.Config) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		w.Write([]byte("sub endpoint ready"))
	}
}

func QRHandler(cfg *config.Config) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		w.Write([]byte("qr endpoint ready"))
	}
}
