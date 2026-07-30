package i18n

import (
	"embed"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

//go:embed zh.json
var zhFS embed.FS

//go:embed en.json
var enFS embed.FS

//go:embed ja.json
var jaFS embed.FS

var zhTable map[string]string

var (
	currentLang string
	tables      map[string]map[string]string
)

func init() {
	zhTable = loadJSON(zhFS, "zh.json")
	tables = make(map[string]map[string]string)
	tables["zh"] = zhTable
	tables["en"] = loadJSON(enFS, "en.json")
	tables["ja"] = loadJSON(jaFS, "ja.json")
}

func loadJSON(fs embed.FS, name string) map[string]string {
	data, err := fs.ReadFile(name)
	if err != nil {
		panic("i18n: cannot embed " + name + ": " + err.Error())
	}
	var m map[string]string
	if err := json.Unmarshal(data, &m); err != nil {
		panic("i18n: invalid JSON in " + name + ": " + err.Error())
	}
	return m
}

func SetLang(lang string) {
	currentLang = lang
}

func Lang() string {
	return currentLang
}

func T(key string, args ...interface{}) string {
	if table, ok := tables[currentLang]; ok {
		if val, ok := table[key]; ok {
			if len(args) > 0 {
				return fmt.Sprintf(val, args...)
			}
			return val
		}
	}
	if table, ok := tables["zh"]; ok {
		if val, ok := table[key]; ok {
			if len(args) > 0 {
				return fmt.Sprintf(val, args...)
			}
			return val
		}
	}
	return key
}

var langDir = "/etc/wwps/aegis"
var langFile = filepath.Join(langDir, ".lang")

func detectLangFromEnv() string {
	if lang := strings.TrimSpace(os.Getenv("WWPS_LANG")); lang != "" {
		return lang
	}
	return ""
}

func readLangFile() string {
	data, err := os.ReadFile(langFile)
	if err != nil {
		return ""
	}
	return strings.TrimSpace(string(data))
}

func writeLangFile(lang string) error {
	if err := os.MkdirAll(langDir, 0o755); err != nil {
		return err
	}
	return os.WriteFile(langFile, []byte(lang+"\n"), 0o644)
}

func InitLang(interactive bool) string {
	if lang := detectLangFromEnv(); lang != "" {
		SetLang(lang)
		return lang
	}
	if lang := readLangFile(); lang != "" {
		SetLang(lang)
		return lang
	}
	if !interactive {
		SetLang("zh")
		return "zh"
	}
	for {
		fmt.Println(T("lang.select"))
		fmt.Println(T("lang.zh"))
		fmt.Println(T("lang.en"))
		fmt.Println(T("lang.ja"))
		fmt.Print("> ")
		var choice string
		fmt.Scanln(&choice)
		var selected string
		switch strings.TrimSpace(choice) {
		case "1":
			selected = "zh"
		case "2":
			selected = "en"
		case "3":
			selected = "ja"
		default:
			continue
		}
		SetLang(selected)
		_ = writeLangFile(selected)
		fmt.Println(T("lang.saved", selected))
		return selected
	}
}
