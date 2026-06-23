package i18n

import (
	"embed"
	"encoding/json"
	"testing"
)

//go:embed zh.json
var testFS embed.FS

func TestZhJSONIsValid(t *testing.T) {
	data, err := testFS.ReadFile("zh.json")
	if err != nil {
		t.Fatalf("zh.json not found: %v", err)
	}
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("zh.json is invalid JSON: %v", err)
	}
	if len(m) == 0 {
		t.Fatal("zh.json is empty")
	}
}
