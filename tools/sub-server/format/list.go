package format

import (
	"encoding/base64"
	"strings"

	pb "github.com/youugiuhiuh/Wuthering_Waves_Private_Server/tools/sub-server/proto/sub"
)

func ToBase64List(configs []*pb.ProxyConfig) string {
	var lines []string
	for _, cfg := range configs {
		if uri := BuildURI(cfg); uri != "" {
			lines = append(lines, uri)
		}
	}
	return base64.StdEncoding.EncodeToString([]byte(strings.Join(lines, "\n")))
}

func ToURIPlain(configs []*pb.ProxyConfig) string {
	var lines []string
	for _, cfg := range configs {
		if uri := BuildURI(cfg); uri != "" {
			lines = append(lines, uri)
		}
	}
	return strings.Join(lines, "\n")
}
