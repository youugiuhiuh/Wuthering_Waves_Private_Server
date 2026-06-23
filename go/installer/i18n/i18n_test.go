package i18n

import (
	"testing"
)

func TestT_Basic(t *testing.T) {
	SetLang("zh")
	got := T("banner.title")
	if got != "WWPS TG Bot 管理工具" {
		t.Errorf(`T("banner.title") with zh = %q, want "WWPS TG Bot 管理工具"`, got)
	}

	SetLang("en")
	got = T("banner.title")
	if got != "WWPS TG Bot Management Tool" {
		t.Errorf(`T("banner.title") with en = %q, want "WWPS TG Bot Management Tool"`, got)
	}

	SetLang("ja")
	got = T("banner.title")
	if got != "WWPS TG Bot 管理ツール" {
		t.Errorf(`T("banner.title") with ja = %q, want "WWPS TG Bot 管理ツール"`, got)
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
