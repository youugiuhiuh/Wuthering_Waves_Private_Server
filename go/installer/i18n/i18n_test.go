package i18n

import (
	"embed"
	"encoding/json"
	"testing"
)

//go:embed zh.json en.json ja.json
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

func TestEnJSONIsValid(t *testing.T) {
	data, err := testFS.ReadFile("en.json")
	if err != nil {
		t.Fatalf("en.json not found: %v", err)
	}
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("en.json is invalid JSON: %v", err)
	}
	if len(m) == 0 {
		t.Fatal("en.json is empty")
	}
}

func TestJaJSONIsValid(t *testing.T) {
	data, err := testFS.ReadFile("ja.json")
	if err != nil {
		t.Fatalf("ja.json not found: %v", err)
	}
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		t.Fatalf("ja.json is invalid JSON: %v", err)
	}
	if len(m) == 0 {
		t.Fatal("ja.json is empty")
	}
}

func TestAllJSONHaveSameKeys(t *testing.T) {
	zh := loadJSONMap(testFS, "zh.json")
	en := loadJSONMap(testFS, "en.json")
	ja := loadJSONMap(testFS, "ja.json")

	for k := range zh {
		if _, ok := en[k]; !ok {
			t.Errorf("en.json missing key: %s", k)
		}
		if _, ok := ja[k]; !ok {
			t.Errorf("ja.json missing key: %s", k)
		}
	}
}

func loadJSONMap(fs embed.FS, name string) map[string]string {
	data, err := fs.ReadFile(name)
	if err != nil {
		panic(err)
	}
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		panic(err)
	}
	return m
}
