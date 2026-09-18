package i18n

import (
	"bytes"
	"embed"
	"encoding/json"
	"strings"
	"testing"
)

func TestT_Basic(t *testing.T) {
	SetLang("zh")
	got := T("banner.title")
	if got != "WWPS Aegis 安装工具" {
		t.Errorf(`T("banner.title") with zh = %q, want "WWPS Aegis 安装工具"`, got)
	}

	SetLang("en")
	got = T("banner.title")
	if got != "WWPS Aegis Installer" {
		t.Errorf(`T("banner.title") with en = %q, want "WWPS Aegis Installer"`, got)
	}

	SetLang("ja")
	got = T("banner.title")
	if got != "WWPS Aegis インストーラー" {
		t.Errorf(`T("banner.title") with ja = %q, want "WWPS Aegis インストーラー"`, got)
	}
}

func TestT_FormatArgs(t *testing.T) {
	SetLang("zh")
	got := T("banner.version", "v3.0.5")
	want := "当前版本: v3.0.5"
	if got != want {
		t.Errorf(`T("banner.version", "v3.0.5") = %q, want %q`, got, want)
	}
}

func TestT_Fallback(t *testing.T) {
	SetLang("en")
	got := T("nonexistent.key")
	if got != "nonexistent.key" {
		t.Errorf("missing key should return the key itself, got %q", got)
	}
}

func TestT_FallbackToChinese(t *testing.T) {
	SetLang("zh")
	got := T("menu.exit")
	if got != "0. 退出" {
		t.Errorf(`T("menu.exit") = %q, want "0. 退出"`, got)
	}
}

func TestSetLang(t *testing.T) {
	SetLang("")
	SetLang("en")
	if Lang() != "en" {
		t.Errorf(`after SetLang("en"), Lang() = %q, want "en"`, Lang())
	}
}

// duplicateKeys reports every top-level key that appears more than once in a
// locale file. loadJSON unmarshals into map[string]string, so duplicates are
// silently collapsed to the last occurrence and TestAllKeysExist cannot see
// them; this walks the raw JSON tokens instead.
func duplicateKeys(t *testing.T, fsys embed.FS, name string) []string {
	t.Helper()

	data, err := fsys.ReadFile(name)
	if err != nil {
		t.Fatalf("read %s: %v", name, err)
	}

	dec := json.NewDecoder(bytes.NewReader(data))
	tok, err := dec.Token()
	if err != nil {
		t.Fatalf("%s: %v", name, err)
	}
	if delim, ok := tok.(json.Delim); !ok || delim != '{' {
		t.Fatalf("%s: expected a top-level object, got %v", name, tok)
	}

	seen := make(map[string]int)
	var dups []string
	for dec.More() {
		keyTok, err := dec.Token()
		if err != nil {
			t.Fatalf("%s: %v", name, err)
		}
		key, ok := keyTok.(string)
		if !ok {
			t.Fatalf("%s: expected a string key, got %v", name, keyTok)
		}
		seen[key]++
		if seen[key] == 2 {
			dups = append(dups, key)
		}

		var value json.RawMessage
		if err := dec.Decode(&value); err != nil {
			t.Fatalf("%s: %v", name, err)
		}
	}

	return dups
}

func TestNoDuplicateKeys(t *testing.T) {
	for _, locale := range []struct {
		fsys embed.FS
		name string
	}{
		{zhFS, "zh.json"},
		{enFS, "en.json"},
		{jaFS, "ja.json"},
	} {
		if dups := duplicateKeys(t, locale.fsys, locale.name); len(dups) > 0 {
			t.Errorf("%s has duplicate keys (loadJSON keeps only the last): %v", locale.name, dups)
		}
	}
}

func TestAllKeysExist(t *testing.T) {
	en := loadJSON(enFS, "en.json")
	ja := loadJSON(jaFS, "ja.json")

	if len(en) == 0 {
		t.Fatal("en.json has zero keys")
	}
	if len(ja) == 0 {
		t.Fatal("ja.json has zero keys")
	}
	for k := range zhTable {
		if _, ok := en[k]; !ok {
			t.Errorf("en.json missing key: %s", k)
		}
		if _, ok := ja[k]; !ok {
			t.Errorf("ja.json missing key: %s", k)
		}
	}
}

// simplex onboarding 文案守卫。
// Task 5 的 main.go 已按这四个 key 名查询，任何一处拼错都会在安装器输出里以裸 key
// 名暴露，而 TestAllKeysExist 只检查 zh ⊆ en/ja，无法发现「三个语言文件同时拼错」。
// 另外 address_ready 是带参文案，必须保留 %s 占位符，否则 i18n.T 的实参会被 fmt 丢弃。
func TestSimplexAddressKeysKeepFormatVerb(t *testing.T) {
	newKeys := []string{
		"simplex.address_ready",
		"simplex.address_paste_hint",
		"simplex.address_pending",
		"simplex.admin_fill_hint",
	}
	for _, locale := range []struct {
		name string
		data map[string]string
	}{
		{"zh.json", loadJSON(zhFS, "zh.json")},
		{"en.json", loadJSON(enFS, "en.json")},
		{"ja.json", loadJSON(jaFS, "ja.json")},
	} {
		for _, key := range newKeys {
			if v, ok := locale.data[key]; !ok || strings.TrimSpace(v) == "" {
				t.Errorf("%s: key %q 缺失或为空（main.go 会按这个名字查找）", locale.name, key)
			}
		}
		if !strings.Contains(locale.data["simplex.address_ready"], "%s") {
			t.Errorf("%s: simplex.address_ready 缺少 %%s 占位符: %q",
				locale.name, locale.data["simplex.address_ready"])
		}
	}
}

// P5 文案守卫：main.go 的 disableSimplexServiceIfPresent 按这两个名字查询，
// 而 TestAllKeysExist 只能发现「zh 有而 en/ja 缺」，发现不了「三个语言文件同时拼错」。
// disable_failed 带 %s，必须保留占位符。
func TestSimplexDisableKeysExist(t *testing.T) {
	keys := []string{"simplex.service_disabled", "simplex.disable_failed"}
	for _, locale := range []struct {
		name string
		data map[string]string
	}{
		{"zh.json", loadJSON(zhFS, "zh.json")},
		{"en.json", loadJSON(enFS, "en.json")},
		{"ja.json", loadJSON(jaFS, "ja.json")},
	} {
		for _, key := range keys {
			if v, ok := locale.data[key]; !ok || strings.TrimSpace(v) == "" {
				t.Errorf("%s: key %q 缺失或为空（main.go 会按这个名字查找）", locale.name, key)
			}
		}
		if !strings.Contains(locale.data["simplex.disable_failed"], "%s") {
			t.Errorf("%s: simplex.disable_failed 缺少 %%s 占位符: %q",
				locale.name, locale.data["simplex.disable_failed"])
		}
	}
}

// P4 文案守卫：installAegis 按这两个名字查询；reconfigure_prompt 带 %s（当前平台），
// 缺占位符会让平台名被 fmt 丢弃。
func TestInstallReconfigureKeysExist(t *testing.T) {
	keys := []string{"install.reconfigure_prompt", "install.reconfigure_warning"}
	for _, locale := range []struct {
		name string
		data map[string]string
	}{
		{"zh.json", loadJSON(zhFS, "zh.json")},
		{"en.json", loadJSON(enFS, "en.json")},
		{"ja.json", loadJSON(jaFS, "ja.json")},
	} {
		for _, key := range keys {
			if v, ok := locale.data[key]; !ok || strings.TrimSpace(v) == "" {
				t.Errorf("%s: key %q 缺失或为空", locale.name, key)
			}
		}
		if !strings.Contains(locale.data["install.reconfigure_prompt"], "%s") {
			t.Errorf("%s: install.reconfigure_prompt 缺少 %%s 占位符: %q",
				locale.name, locale.data["install.reconfigure_prompt"])
		}
	}
}
