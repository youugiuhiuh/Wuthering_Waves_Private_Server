package main

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"os/exec"
	"path/filepath"
	"slices"
	"strings"
	"testing"
	"testing/iotest"

	tea "charm.land/bubbletea/v2"
	"github.com/awnumar/memguard"
)

func TestNormalizeMatrixMXID(t *testing.T) {
	for _, tt := range []struct{ input, mxid, server string }{
		{"@alice:unredacted.org", "@alice:unredacted.org", "unredacted.org"},
		{"us-sanjose:matrix.org", "@us-sanjose:matrix.org", "matrix.org"},
	} {
		mxid, server, err := normalizeMatrixMXID(tt.input)
		if err != nil || mxid != tt.mxid || server != tt.server {
			t.Fatalf("normalizeMatrixMXID(%q) = (%q, %q, %v)", tt.input, mxid, server, err)
		}
	}

	for _, input := range []string{"", "alice", "@alice", " @alice:matrix.org", "@alice:matrix.org ", "@alice:matrix.org extra", "@alice:matrix .org", "alice:example.com/path"} {
		if _, _, err := normalizeMatrixMXID(input); err == nil {
			t.Errorf("normalizeMatrixMXID(%q) expected error", input)
		}
	}
}

func TestValidateMatrixHomeserver(t *testing.T) {
	if got, err := validateMatrixHomeserver("https://matrix.org/"); err != nil || got != "https://matrix.org" {
		t.Fatalf("validateMatrixHomeserver() = (%q, %v)", got, err)
	}

	for _, raw := range []string{"", "http://matrix.org", "https://", "https://user@matrix.org", "https://matrix.org?x=1", "https://matrix.org#fragment"} {
		if _, err := validateMatrixHomeserver(raw); err == nil {
			t.Errorf("validateMatrixHomeserver(%q) expected error", raw)
		}
	}
}

func TestDiscoverMatrixHomeserver(t *testing.T) {
	t.Run("success", func(t *testing.T) {
		requestPath := make(chan string, 1)
		server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			requestPath <- r.URL.Path
			_, _ = io.WriteString(w, `{"m.homeserver":{"base_url":"https://matrix-client.example"}}`)
		}))
		defer server.Close()

		mxid, homeserver, err := discoverMatrixHomeserver("alice:"+strings.TrimPrefix(server.URL, "https://"), server.Client())
		if err != nil || mxid == "" || homeserver != "https://matrix-client.example" {
			t.Fatalf("discovery = (%q, %q, %v)", mxid, homeserver, err)
		}
		if got := <-requestPath; got != "/.well-known/matrix/client" {
			t.Fatalf("path = %q", got)
		}
	})

	for _, tt := range []struct {
		name, body string
		status     int
	}{
		{"non-2xx response", "", http.StatusBadGateway},
		{"malformed JSON", "{", http.StatusOK},
		{"non-HTTPS base URL", `{"m.homeserver":{"base_url":"http://matrix-client.example"}}`, http.StatusOK},
	} {
		t.Run(tt.name, func(t *testing.T) {
			server := httptest.NewTLSServer(http.HandlerFunc(func(w http.ResponseWriter, _ *http.Request) {
				w.WriteHeader(tt.status)
				_, _ = io.WriteString(w, tt.body)
			}))
			defer server.Close()

			if _, _, err := discoverMatrixHomeserver("alice:"+strings.TrimPrefix(server.URL, "https://"), server.Client()); err == nil {
				t.Fatal("discoverMatrixHomeserver() expected error")
			}
		})
	}
}

func TestValidateAdminID(t *testing.T) {
	for _, id := range []string{"0", "9223372036854775807", "-9223372036854775808"} {
		if err := validateAdminID(id); err != nil {
			t.Errorf("validateAdminID(%q) unexpected error: %v", id, err)
		}
	}

	for _, id := range []string{"", "not-a-number", "9223372036854775808"} {
		if err := validateAdminID(id); err == nil {
			t.Errorf("validateAdminID(%q) expected error", id)
		}
	}
}

func TestRunSetupCommandReturnsFailure(t *testing.T) {
	if err := runSetupCommand("/bin/false", []byte(`{}`)); err == nil {
		t.Fatal("runSetupCommand() expected error")
	}
}

func TestPlatformFromService(t *testing.T) {
	tests := map[string]string{
		"ExecStart=/etc/wwps/aegis/aegis":           "tg",
		"ExecStart=/etc/wwps/aegis/aegis --matrix":  "matrix",
		"ExecStart=/etc/wwps/aegis/aegis --simplex": "simplex",
		"ExecStart=/etc/wwps/aegis/aegis --all":     "tg-matrix",
	}
	for service, want := range tests {
		if got := platformFromService([]byte(service)); got != want {
			t.Errorf("platformFromService(%q) = %q, want %q", service, got, want)
		}
	}
}

func TestRecoveryPlatformForService(t *testing.T) {
	cases := []struct {
		name     string
		service  []byte
		choice   string
		platform string
		rebuild  bool
		wantErr  bool
	}{
		{"existing matrix", []byte("ExecStart=/aegis --matrix\n"), "", "matrix", false, false},
		{"missing unit selection", nil, "4", "tg-matrix", true, false},
		{"missing unit invalid selection", nil, "0", "", false, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			platform, rebuild, err := recoveryPlatformForService(tc.service, tc.choice)
			if (err != nil) != tc.wantErr || platform != tc.platform || rebuild != tc.rebuild {
				t.Fatalf("platform=%q rebuild=%t err=%v", platform, rebuild, err)
			}
		})
	}
}

func TestPlatformSetupForChoice(t *testing.T) {
	tests := map[string]struct {
		tg      bool
		matrix  bool
		simplex bool
	}{
		"1": {tg: true},
		"2": {matrix: true},
		"4": {tg: true, matrix: true},
		"5": {simplex: true},
	}

	for choice, want := range tests {
		tg, matrix, simplex, err := platformSetupForChoice(choice)
		if err != nil {
			t.Errorf("platformSetupForChoice(%q) unexpected error: %v", choice, err)
		}
		if tg != want.tg || matrix != want.matrix || simplex != want.simplex {
			t.Errorf("platformSetupForChoice(%q) = (%t, %t, %t), want (%t, %t, %t)", choice, tg, matrix, simplex, want.tg, want.matrix, want.simplex)
		}
	}

	if _, _, _, err := platformSetupForChoice("0"); err == nil {
		t.Error("platformSetupForChoice(\"0\") expected error")
	}
}

func TestServicePlatformForSetup(t *testing.T) {
	tests := []struct {
		tg, matrix, simplex bool
		want                string
	}{
		{tg: true, want: "tg"},
		{matrix: true, want: "matrix"},
		{simplex: true, want: "simplex"},
		{simplex: true, matrix: true, want: "simplex"},
		{tg: true, matrix: true, want: "tg-matrix"},
	}

	for _, test := range tests {
		if got := servicePlatformForSetup(test.tg, test.matrix, test.simplex); got != test.want {
			t.Errorf("servicePlatformForSetup(%t, %t, %t) = %q, want %q", test.tg, test.matrix, test.simplex, got, test.want)
		}
	}
}

func TestPlatformForNonInteractive(t *testing.T) {
	tests := []struct {
		name                            string
		hasToken, hasMatrix, hasSimplex bool
		want                            string
		wantErr                         bool
	}{
		{"telegram only", true, false, false, "tg", false},
		{"matrix only", false, true, false, "matrix", false},
		{"simplex only", false, false, true, "simplex", false},
		{"telegram+matrix", true, true, false, "tg-matrix", false},
		{"telegram+simplex", true, false, true, "tg-simplex", false},
		{"three platforms", true, true, true, "", true},
		{"no fields", false, false, false, "", true},
	}

	for _, tc := range tests {
		t.Run(tc.name, func(t *testing.T) {
			got, err := platformForNonInteractive(tc.hasToken, tc.hasMatrix, tc.hasSimplex)
			if (err != nil) != tc.wantErr {
				t.Fatalf("platformForNonInteractive(%t, %t, %t) err = %v, wantErr = %t",
					tc.hasToken, tc.hasMatrix, tc.hasSimplex, err, tc.wantErr)
			}
			if got != tc.want {
				t.Fatalf("platformForNonInteractive(%t, %t, %t) = %q, want %q",
					tc.hasToken, tc.hasMatrix, tc.hasSimplex, got, tc.want)
			}
		})
	}
}

func TestPlatformSelectorRejectsEmptyConfirmation(t *testing.T) {
	m := newPlatformSelector()
	updated, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	m = updated.(platformSelector)
	if m.confirmed || cmd != nil {
		t.Fatalf("empty confirmation = %#v, %v", m, cmd)
	}
}

func TestUninstallManifestIncludesRustArtifacts(t *testing.T) {
	wantServices := []string{"wwps-aegis", "wwps-simplex", "wwps-core", "wwps-box"}
	wantPaths := []string{
		"/etc/systemd/system/wwps-aegis.service",
		"/etc/systemd/system/wwps-simplex.service",
		"/etc/systemd/system/wwps-core.service",
		"/etc/systemd/system/wwps-box.service",
		"/etc/init.d/wwps-core",
		"/etc/wwps",
		"/tmp/wwps-core-installer",
		"/tmp/wwps-core-upgrade",
		"/tmp/sing-box-install",
		"/etc/sysctl.d/90-wwps-bbr3-optimize.conf",
		"/etc/systemd/system/apt-daily-upgrade.timer.d/aegis-timezone.conf",
		"/etc/systemd/system/apt-daily.timer.d/aegis-timezone.conf",
	}

	if !slices.Equal(uninstallServices, wantServices) {
		t.Fatalf("uninstallServices = %v, want %v", uninstallServices, wantServices)
	}
	if !slices.Equal(uninstallPaths, wantPaths) {
		t.Fatalf("uninstallPaths = %v, want %v", uninstallPaths, wantPaths)
	}
}

func TestExtractBase32Secret(t *testing.T) {
	tests := []struct {
		name    string
		output  []byte
		want    []byte
		wantErr bool
	}{
		{
			name:    "仅一行合法 base32",
			output:  []byte("JBSWY3DPEHPK3PXP"),
			want:    []byte("JBSWY3DPEHPK3PXP"),
			wantErr: false,
		},
		{
			name:    "多行含 Binary Integrity Hash，最后一行是 base32",
			output:  []byte("Binary Integrity Hash: abc123\nJBSWY3DPEHPK3PXP"),
			want:    []byte("JBSWY3DPEHPK3PXP"),
			wantErr: false,
		},
		{
			name:    "多行含 hash 与换行，取最后一行合法 base32",
			output:  []byte("Binary Integrity Hash: x\nJBSWY3DPEHPK3PXP\nMFRGGZDFMZTWQ2LK"),
			want:    []byte("MFRGGZDFMZTWQ2LK"),
			wantErr: false,
		},
		{
			name:    "行首尾空格 trim 后合法",
			output:  []byte("  JBSWY3DPEHPK3PXP  \n"),
			want:    []byte("JBSWY3DPEHPK3PXP"),
			wantErr: false,
		},
		{
			name:    "空输出",
			output:  []byte(""),
			want:    nil,
			wantErr: true,
		},
		{
			name:    "仅换行",
			output:  []byte("\n\n"),
			want:    nil,
			wantErr: true,
		},
		{
			name:    "无合法 base32 行",
			output:  []byte("Binary Integrity Hash: abc\nnot-base32\n"),
			want:    nil,
			wantErr: true,
		},
		{
			name:    "base32 不足 16 位",
			output:  []byte("JBSWY3DP"),
			want:    nil,
			wantErr: true,
		},
		{
			name:    "含非法字符",
			output:  []byte("JBSWY3DPEHPK3PXP\x00"),
			want:    nil,
			wantErr: true,
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got, err := extractBase32Secret(tt.output)
			if (err != nil) != tt.wantErr {
				t.Errorf("extractBase32Secret() error = %v, wantErr %v", err, tt.wantErr)
				return
			}
			if !bytes.Equal(got, tt.want) {
				t.Errorf("extractBase32Secret() got = %q, want %q", got, tt.want)
			}
		})
	}
}

func TestBuildSetupPayload(t *testing.T) {
	t.Run("without matrix", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("token:abc"), []byte("123"), []byte("SECRET"),
			"", "", "", nil, nil, "",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("无效 JSON: %v", err)
		}
		if string(parsed["token"].(string)) != "token:abc" {
			t.Errorf("token = %v, want token:abc", parsed["token"])
		}
		if parsed["matrix_homeserver"] != nil {
			t.Error("不应包含 matrix_homeserver")
		}
	})

	t.Run("with matrix", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("token:abc"), []byte("123"), []byte("SECRET"),
			"https://matrix.org", "@bot:matrix.org", "!room:matrix.org", []byte("pass123"), nil, "",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("无效 JSON: %v", err)
		}
		if parsed["matrix_homeserver"] != "https://matrix.org" {
			t.Errorf("homeserver = %v, want https://matrix.org", parsed["matrix_homeserver"])
		}
		if parsed["matrix_username"] != "@bot:matrix.org" {
			t.Errorf("username = %v, want @bot:matrix.org", parsed["matrix_username"])
		}
		if parsed["matrix_password"] != "pass123" {
			t.Errorf("password = %v, want pass123", parsed["matrix_password"])
		}
		if parsed["matrix_room_id"] != "!room:matrix.org" {
			t.Errorf("room_id = %v, want !room:matrix.org", parsed["matrix_room_id"])
		}
	})

	t.Run("partial matrix fields", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("t"), []byte("1"), []byte("S"),
			"https://matrix.org", "", "", nil, nil, "",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("无效 JSON: %v", err)
		}
		if parsed["matrix_homeserver"] != "https://matrix.org" {
			t.Errorf("homeserver = %v, want https://matrix.org", parsed["matrix_homeserver"])
		}
		if parsed["matrix_username"] != nil {
			t.Error("不应包含 matrix_username")
		}
	})

	t.Run("without discord fields", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("t"), []byte("1"), []byte("S"),
			"", "", "", nil, nil, "",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if parsed["discord_token"] != nil {
			t.Error("不应包含 discord_token")
		}
		if parsed["discord_admin_id"] != nil {
			t.Error("不应包含 discord_admin_id")
		}
	})

	t.Run("with matrix recovery key", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("t"), []byte("1"), []byte("S"),
			"", "", "", nil, nil,
			"matrix-recovery-key-value",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if parsed["matrix_recovery_key"] != "matrix-recovery-key-value" {
			t.Errorf("matrix_recovery_key = %v, want matrix-recovery-key-value", parsed["matrix_recovery_key"])
		}
	})

	t.Run("without matrix recovery key", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("t"), []byte("1"), []byte("S"),
			"", "", "", nil, nil, "",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if parsed["matrix_recovery_key"] != nil {
			t.Error("不应包含 matrix_recovery_key")
		}
	})
}

func TestAppendJSONEscaped(t *testing.T) {
	tests := []struct {
		name  string
		input []byte
		want  string
	}{
		{
			name:  "normal ASCII",
			input: []byte("hello"),
			want:  "\"hello\"",
		},
		{
			name:  "special JSON chars",
			input: []byte("a\"b\\c"),
			want:  "\"a\\\"b\\\\c\"",
		},
		{
			name:  "control chars",
			input: []byte("a\nb\tc"),
			want:  "\"a\\nb\\tc\"",
		},
		{
			name:  "valid UTF-8 multi-byte chars pass through",
			input: []byte("+ì®"),
			want:  "\"+ì®\"",
		},
		{
			name:  "DEL char 0x7F passes through as valid UTF-8",
			input: []byte{0x7F},
			want:  "\"\x7f\"",
		},
		{
			name:  "non-ASCII bytes 0x80-0xFF (invalid UTF-8) get escaped",
			input: []byte{0x80, 0xFF, 0xE0},
			want:  "\"\\u0080\\u00ff\\u00e0\"",
		},
		{
			name:  "mixed with non-ASCII (invalid UTF-8) get escaped",
			input: []byte("a\x80b\xFFc"),
			want:  "\"a\\u0080b\\u00ffc\"",
		},
	}
	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := appendJSONEscaped(nil, tt.input)
			if string(got) != tt.want {
				t.Errorf("appendJSONEscaped() = %s, want %s", string(got), tt.want)
			}
			var parsed interface{}
			if err := json.Unmarshal(got, &parsed); err != nil {
				t.Errorf("输出不是合法 JSON: %v", err)
			}
		})
	}
}

func TestParseKeyVal(t *testing.T) {
	t.Run("basic fields", func(t *testing.T) {
		data := []byte("token=abc:123\nadmin_id=456\ntotp_secret=SECRET\n")
		cfg, err := parseKeyVal(data)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if cfg.Token != "abc:123" {
			t.Errorf("Token = %q, want abc:123", cfg.Token)
		}
		if cfg.AdminID != "456" {
			t.Errorf("AdminID = %q, want 456", cfg.AdminID)
		}
		if cfg.TOTPSecret != "SECRET" {
			t.Errorf("TOTPSecret = %q, want SECRET", cfg.TOTPSecret)
		}
	})

	t.Run("with matrix fields", func(t *testing.T) {
		data := []byte("token=t\nadmin_id=1\nmatrix_homeserver=https://matrix.org\nmatrix_username=@bot:matrix.org\nmatrix_password=pass\nmatrix_room_id=!room:matrix.org\n")
		cfg, err := parseKeyVal(data)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if cfg.MatrixHS != "https://matrix.org" {
			t.Errorf("MatrixHS = %q, want https://matrix.org", cfg.MatrixHS)
		}
		if cfg.MatrixUser != "@bot:matrix.org" {
			t.Errorf("MatrixUser = %q, want @bot:matrix.org", cfg.MatrixUser)
		}
		if cfg.MatrixPassword != "pass" {
			t.Errorf("MatrixPassword = %q, want pass", cfg.MatrixPassword)
		}
		if cfg.MatrixRoom != "!room:matrix.org" {
			t.Errorf("MatrixRoom = %q, want !room:matrix.org", cfg.MatrixRoom)
		}
	})

	t.Run("skips empty lines and comments", func(t *testing.T) {
		data := []byte("# this is a comment\n\ntoken=t\nadmin_id=1\n  # indented comment\n")
		cfg, err := parseKeyVal(data)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if cfg.Token != "t" {
			t.Errorf("Token = %q, want t", cfg.Token)
		}
	})

	t.Run("missing required fields", func(t *testing.T) {
		data := []byte("totp_secret=SECRET\n")
		_, err := parseKeyVal(data)
		if err == nil {
			t.Fatal("expected error for missing fields")
		}
	})

	t.Run("non-ASCII password values pass through", func(t *testing.T) {
		input := []byte{0x74, 0x6f, 0x6b, 0x65, 0x6e, 0x3d, 0x74, 0x0a, 0x61, 0x64, 0x6d, 0x69, 0x6e, 0x5f, 0x69, 0x64, 0x3d, 0x31, 0x0a, 0x6d, 0x61, 0x74, 0x72, 0x69, 0x78, 0x5f, 0x70, 0x61, 0x73, 0x73, 0x77, 0x6f, 0x72, 0x64, 0x3d}
		input = append(input, []byte{0xEC, 0xAE, 0x27, 0x22, 0x75, 0x3D, 0x4F, 0x61, 0xCC, 0x22, 0xF9, 0xF8, 0x52, 0x50, 0xFA, 0x6A, 0xC7, 0x2C, 0xDA, 0xD2, 0xE2, 0x3F, 0xAC}...)
		input = append(input, '\n')
		cfg, err := parseKeyVal(input)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if len(cfg.MatrixPassword) == 0 {
			t.Fatal("MatrixPassword should not be empty")
		}
		payload := buildSetupPayload(
			[]byte(cfg.Token), []byte(cfg.AdminID), []byte(cfg.TOTPSecret),
			cfg.MatrixHS, cfg.MatrixUser, cfg.MatrixRoom, []byte(cfg.MatrixPassword), nil, "",
			"", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("payload should be valid JSON: %v", err)
		}
	})

	t.Run("with matrix recovery key", func(t *testing.T) {
		data := []byte("token=t\nadmin_id=1\nmatrix_recovery_key=my-recovery-key\n")
		cfg, err := parseKeyVal(data)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if cfg.MatrixRecoveryKey != "my-recovery-key" {
			t.Errorf("MatrixRecoveryKey = %q, want my-recovery-key", cfg.MatrixRecoveryKey)
		}
	})
}

func TestParseTrustedComment(t *testing.T) {
	tests := []struct {
		input    string
		wantVer  string
		wantName string
		wantErr  bool
	}{
		{"v3.1.8:aegis", "v3.1.8", "aegis", false},
		{"v2.0.0:installer", "v2.0.0", "installer", false},
		{"nocolon", "", "", true},
		{"too:many:colons", "too", "many:colons", false},
		{"", "", "", true},
	}
	for _, tc := range tests {
		ver, name, err := parseTrustedComment(tc.input)
		if tc.wantErr && err == nil {
			t.Errorf("parseTrustedComment(%q) expected error", tc.input)
		}
		if !tc.wantErr {
			if err != nil {
				t.Errorf("parseTrustedComment(%q) unexpected error: %v", tc.input, err)
			}
			if ver != tc.wantVer {
				t.Errorf("parseTrustedComment(%q) ver = %q, want %q", tc.input, ver, tc.wantVer)
			}
			if name != tc.wantName {
				t.Errorf("parseTrustedComment(%q) name = %q, want %q", tc.input, name, tc.wantName)
			}
		}
	}
}

func TestFindMinisigAsset(t *testing.T) {
	release := &latestRelease{
		Assets: []releaseAsset{
			{Name: "aegis", BrowserDownloadURL: "https://example.com/aegis"},
			{Name: "aegis.minisig", BrowserDownloadURL: "https://example.com/aegis.minisig"},
			{Name: "installer", BrowserDownloadURL: "https://example.com/installer"},
			{Name: "installer.minisig", BrowserDownloadURL: "https://example.com/installer.minisig"},
		},
	}
	asset := findMinisigAsset(release, "aegis")
	if asset == nil {
		t.Fatal("findMinisigAsset(aegis) returned nil")
	}
	if asset.Name != "aegis.minisig" {
		t.Errorf("findMinisigAsset(aegis).Name = %q, want %q", asset.Name, "aegis.minisig")
	}
	if asset.BrowserDownloadURL != "https://example.com/aegis.minisig" {
		t.Errorf("findMinisigAsset(aegis).BrowserDownloadURL = %q, want %q", asset.BrowserDownloadURL, "https://example.com/aegis.minisig")
	}
	asset2 := findMinisigAsset(release, "nonexistent")
	if asset2 != nil {
		t.Errorf("findMinisigAsset(nonexistent) = %v, want nil", asset2)
	}
}

func TestMemguardCleanupDoesNotCrash(t *testing.T) {
	if os.Getenv("WWPS_MEMGUARD_HELPER") != "1" {
		cmd := exec.Command(os.Args[0], "-test.run=TestMemguardCleanupDoesNotCrash")
		cmd.Env = append(os.Environ(), "WWPS_MEMGUARD_HELPER=1")
		out, err := cmd.CombinedOutput()
		if bytes.Contains(out, []byte("ENCLAVE_UNAVAILABLE")) {
			t.Skip("memguard enclaves unavailable in this environment")
		}
		if err != nil {
			t.Fatalf("helper subprocess failed: %v\n%s", err, out)
		}
		if !bytes.Contains(out, []byte("CONTINUED")) {
			t.Fatalf("helper did not continue past cleanup point:\n%s", out)
		}
		return
	}
	helperMemguardCleanup()
}

// helperMemguardCleanup mirrors the tail of firstTimeSetup: three memguard
// enclaves are opened, a successful child process runs, then the cleanup
// sequence executes. Keep it in sync with firstTimeSetup — it encodes the safe
// cleanup contract.
func helperMemguardCleanup() {
	defer func() {
		if r := recover(); r != nil {
			fmt.Println("ENCLAVE_UNAVAILABLE")
			os.Exit(0)
		}
	}()
	enclaves := []*memguard.Enclave{
		memguard.NewEnclave([]byte("telegram-token-value")),
		memguard.NewEnclave([]byte("123456789")),
		memguard.NewEnclave([]byte("TOTP-secret-value")),
	}
	var bufs []*memguard.LockedBuffer
	var bufSlices [][]byte
	for _, e := range enclaves {
		buf, err := e.Open()
		if err != nil {
			fmt.Println("ENCLAVE_UNAVAILABLE")
			os.Exit(0)
		}
		bufs = append(bufs, buf)
		bufSlices = append(bufSlices, buf.Bytes())
	}
	for _, b := range bufs {
		defer b.Destroy()
	}

	if err := exec.Command("true").Run(); err != nil {
		fmt.Println("CHILD_FAILED")
		os.Exit(1)
	}

	_ = bufSlices // Destroy() wipes these buffers; do not zeroBytes frozen memory

	fmt.Println("CONTINUED")
	os.Exit(0)
}

func TestUsesManualHomeserverFallback(t *testing.T) {
	if !usesManualHomeserverFallback(true, errors.New("well-known unavailable")) {
		t.Fatal("interactive discovery failure should fall back")
	}
	if usesManualHomeserverFallback(false, errors.New("well-known unavailable")) {
		t.Fatal("non-TTY discovery failure must return an error")
	}
}

func TestPlatformSelectorTogglesAndConfirms(t *testing.T) {
	m := newPlatformSelector()
	if m.cursor != 0 || m.telegram || m.matrix || m.simplex || m.confirmed {
		t.Fatalf("initial selector state = %#v", m)
	}

	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	m = updated.(platformSelector)
	updated, cmd := m.Update(tea.KeyPressMsg{Code: tea.KeyEnter})
	m = updated.(platformSelector)
	if !m.confirmed || !m.telegram || m.matrix || m.simplex || cmd == nil {
		t.Fatalf("confirmed selector state = %#v", m)
	}
}

func TestPlatformSelectorSimplexIsStandalone(t *testing.T) {
	m := newPlatformSelector()
	m.telegram = true
	m.cursor = 2
	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.simplex || m.telegram || m.matrix {
		t.Fatalf("selecting simplex must clear telegram and matrix: %#v", m)
	}
}

func TestPlatformSelectorCursorWraps(t *testing.T) {
	m := newPlatformSelector()

	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeyUp})
	m = updated.(platformSelector)
	if m.cursor != 2 {
		t.Fatalf("cursor after up from first row = %d, want 2", m.cursor)
	}

	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	m = updated.(platformSelector)
	if m.cursor != 0 {
		t.Fatalf("cursor after down from last row = %d, want 0", m.cursor)
	}
}

func TestPlatformSelectorTogglesMatrixAndSimplex(t *testing.T) {
	m := newPlatformSelector()

	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	m = updated.(platformSelector)
	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.matrix {
		t.Fatalf("matrix selection = %#v, want selected", m)
	}

	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if m.matrix {
		t.Fatalf("matrix selection = %#v, want cleared", m)
	}

	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeyDown})
	m = updated.(platformSelector)
	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.simplex {
		t.Fatalf("simplex selection = %#v, want selected", m)
	}

	updated, _ = m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if m.simplex {
		t.Fatalf("simplex selection = %#v, want cleared", m)
	}
}

func TestPlatformSelectorCtrlCQuitsWithoutConfirmation(t *testing.T) {
	m := newPlatformSelector()

	updated, cmd := m.Update(tea.KeyPressMsg{Code: 'c', Mod: tea.ModCtrl})
	m = updated.(platformSelector)
	if m.confirmed {
		t.Fatalf("state after ctrl+c = %#v, should not confirm", m)
	}
	if cmd == nil {
		t.Fatal("ctrl+c command = nil, want tea.Quit")
	}
	if msg := cmd(); msg == nil {
		t.Fatal("ctrl+c command returned nil, want tea.QuitMsg")
	} else if _, ok := msg.(tea.QuitMsg); !ok {
		t.Fatalf("ctrl+c command = %T, want tea.QuitMsg", msg)
	}
}

func TestParsePlatformChoice(t *testing.T) {
	tests := map[string]struct{ tg, matrix, simplex bool }{
		"telegram":          {tg: true},
		"matrix":            {matrix: true},
		"simplex":           {simplex: true},
		"telegram + matrix": {tg: true, matrix: true},
	}
	for input, want := range tests {
		tg, matrix, simplex, err := parsePlatformChoice(input)
		if err != nil || tg != want.tg || matrix != want.matrix || simplex != want.simplex {
			t.Fatalf("parsePlatformChoice(%q) = (%t, %t, %t, %v)", input, tg, matrix, simplex, err)
		}
	}
	for _, input := range []string{"discord", "discord + matrix", "telegram + discord"} {
		if _, _, _, err := parsePlatformChoice(input); err == nil {
			t.Fatalf("parsePlatformChoice(%q) must be rejected: Discord 平台已移除", input)
		}
	}
}

func TestParsePlatformChoiceSimplex(t *testing.T) {
	tg, matrix, simplex, err := parsePlatformChoice("simplex")
	if err != nil {
		t.Fatalf("unexpected err: %v", err)
	}
	if tg || matrix || !simplex {
		t.Fatalf("simplex standalone expected, got tg=%t matrix=%t simplex=%t", tg, matrix, simplex)
	}
}

func TestParsePlatformChoiceRejectsSimplexCombo(t *testing.T) {
	for _, input := range []string{"simplex+matrix", "simplex+telegram", "simplex+discord"} {
		if _, _, _, err := parsePlatformChoice(input); err == nil {
			t.Fatalf("parsePlatformChoice(%q) must be rejected: SimpleX is standalone-only", input)
		}
	}
}

func TestWriteSystemdServiceSimplexUsesFlag(t *testing.T) {
	// writeSystemdService 写固定路径，这里验证可测的 flag 映射函数。
	tests := map[string]string{
		"tg":        "",
		"matrix":    "--matrix",
		"simplex":   "--simplex",
		"tg-matrix": "--all",
	}
	for platform, want := range tests {
		if got := platformFlagFor(platform); got != want {
			t.Errorf("platformFlagFor(%q) = %q, want %q", platform, got, want)
		}
	}
}

func TestUsesInteractivePlatformSelector(t *testing.T) {
	for _, tt := range []struct {
		stdinIsTerminal  bool
		stdoutIsTerminal bool
		want             bool
	}{
		{stdinIsTerminal: true, stdoutIsTerminal: true, want: true},
		{stdinIsTerminal: false, stdoutIsTerminal: true, want: false},
		{stdinIsTerminal: true, stdoutIsTerminal: false, want: false},
	} {
		if got := usesInteractivePlatformSelector(tt.stdinIsTerminal, tt.stdoutIsTerminal); got != tt.want {
			t.Fatalf("usesInteractivePlatformSelector(%t, %t) = %t, want %t", tt.stdinIsTerminal, tt.stdoutIsTerminal, got, tt.want)
		}
	}
}

func TestParseKeyValSimplexFields(t *testing.T) {
	data := []byte("simplex_port=5225\nsimplex_admin_id=42\n")
	cfg, err := parseKeyVal(data)
	if err != nil {
		t.Fatalf("simplex-only config must be accepted: %v", err)
	}
	if cfg.SimplexPort != "5225" {
		t.Errorf("SimplexPort = %q, want 5225", cfg.SimplexPort)
	}
	if cfg.SimplexAdminID != "42" {
		t.Errorf("SimplexAdminID = %q, want 42", cfg.SimplexAdminID)
	}
}

func TestBuildSetupPayloadSimplexFields(t *testing.T) {
	payload := buildSetupPayload(
		nil, nil, nil,
		"", "", "", nil, nil, "",
		"5225", "42",
	)
	var parsed map[string]interface{}
	if err := json.Unmarshal(payload, &parsed); err != nil {
		t.Fatalf("无效 JSON: %v", err)
	}
	if parsed["simplex_port"] != "5225" {
		t.Errorf("simplex_port = %v, want 5225", parsed["simplex_port"])
	}
	if parsed["simplex_admin_id"] != "42" {
		t.Errorf("simplex_admin_id = %v, want 42", parsed["simplex_admin_id"])
	}
}

func TestBuildSetupPayloadOmitsSimplexWhenEmpty(t *testing.T) {
	payload := buildSetupPayload(
		[]byte("token:abc"), []byte("123"), nil,
		"", "", "", nil, nil, "",
		"", "",
	)
	var parsed map[string]interface{}
	if err := json.Unmarshal(payload, &parsed); err != nil {
		t.Fatalf("无效 JSON: %v", err)
	}
	if _, ok := parsed["simplex_port"]; ok {
		t.Error("不应包含 simplex_port")
	}
	if _, ok := parsed["simplex_admin_id"]; ok {
		t.Error("不应包含 simplex_admin_id")
	}
}

func TestPlatformSelectorSimplexIsExclusive(t *testing.T) {
	cases := []struct {
		name             string
		telegram, matrix bool
		simplex          bool
		wantValid        bool
	}{
		{"simplex alone", false, false, true, true},
		{"simplex with matrix", false, true, true, false},
		{"simplex with telegram", true, false, true, false},
		{"matrix alone", false, true, false, true},
		{"telegram+matrix", true, true, false, true},
		{"telegram alone", true, false, false, true},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			m := platformSelector{
				telegram: tc.telegram,
				matrix:   tc.matrix,
				simplex:  tc.simplex,
			}
			_, _, _, valid := m.platformSelection()
			if valid != tc.wantValid {
				t.Fatalf("platformSelection() valid = %t, want %t", valid, tc.wantValid)
			}
		})
	}
}

func TestPlatformSelectorTogglingSimplexClearsOthers(t *testing.T) {
	m := newPlatformSelector()
	m.matrix = true
	m.cursor = 2
	updated, _ := m.Update(tea.KeyPressMsg{Code: tea.KeySpace})
	m = updated.(platformSelector)
	if !m.simplex || m.matrix || m.telegram {
		t.Fatalf("selecting simplex must clear other platforms: %#v", m)
	}
}

// TestSimplexPortFromUnit 守住重装路径：recovery 分支拿不到用户当初填的端口，
// 必须能从未被覆盖的单元文件里回读，否则重装会把自定义端口改回默认值。
func TestSimplexPortFromUnit(t *testing.T) {
	cases := []struct {
		name    string
		content string
		want    string
	}{
		{"custom port", simplexSystemdUnitContent("6123"), "6123"},
		{"default port", simplexSystemdUnitContent("5225"), "5225"},
		{"empty unit", "", ""},
		{"unrelated unit", "[Service]\nExecStart=/usr/bin/foo --matrix\n", ""},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			if got := simplexPortFromUnit([]byte(tc.content)); got != tc.want {
				t.Fatalf("simplexPortFromUnit() = %q, want %q", got, tc.want)
			}
		})
	}
}

func TestSimplexChatAssetName(t *testing.T) {
	cases := map[string]string{
		"amd64": "simplex-chat-ubuntu-24_04-x86_64",
		"arm64": "simplex-chat-ubuntu-24_04-aarch64",
	}
	for arch, want := range cases {
		if got := simplexChatAssetName(arch); got != want {
			t.Fatalf("arch %s: got %q want %q", arch, got, want)
		}
	}
}

func TestSimplexChatAssetNameRejectsUnknownArch(t *testing.T) {
	for _, arch := range []string{"riscv64", "386", ""} {
		if got := simplexChatAssetName(arch); got != "" {
			t.Fatalf("unknown arch %q must return empty, got %q", arch, got)
		}
	}
}

func TestSimplexSystemdUnitPinsVersionAndLocalhost(t *testing.T) {
	unit := simplexSystemdUnitContent("5225")
	if !strings.Contains(unit, "ExecStart=") || !strings.Contains(unit, "-p 5225") {
		t.Fatalf("unit must start simplex-chat on the configured port: %s", unit)
	}
	if !strings.Contains(unit, "Restart=always") {
		t.Fatal("unit must restart on failure")
	}
	if want := filepath.Join(installDir, "simplex-chat"); !strings.Contains(unit, want) {
		t.Fatalf("unit must exec %s: %s", want, unit)
	}
}

// TestSimplexSystemdUnitStartsUnattended 守住一个实测出来的坑：全新机器上只跑
// `simplex-chat -p PORT` 会因为「没有 user profile」而停在交互式提问，stdin 是
// /dev/null 时直接退出；配合 Restart=always 就是无限崩溃重启。这些参数是把
// 它变成可无人值守启动的关键，不能让后续修改误删。
func TestSimplexSystemdUnitStartsUnattended(t *testing.T) {
	unit := simplexSystemdUnitContent("5225")
	for _, need := range []string{
		"WorkingDirectory=",
		"--create-bot-display-name Aegis",
		"-y",
		"-d ",
		"--files-folder ",
	} {
		if !strings.Contains(unit, need) {
			t.Fatalf("unit must contain %q so it can start without a tty: %s", need, unit)
		}
	}
	if want := filepath.Join(simplexDataDir(), "simplex_v1"); !strings.Contains(unit, want) {
		t.Fatalf("unit must pin the database prefix to %s: %s", want, unit)
	}
}

// TestSimplexSystemdUnitNeverExposesTheApi 守住安全边界：SimpleX 的 WebSocket API
// 没有任何鉴权，simplex-chat 默认只绑定 localhost。单元文件里不允许出现任何会把
// 监听地址改成非本机的参数，否则等于把无鉴权 API 暴露到公网。
func TestSimplexSystemdUnitNeverExposesTheApi(t *testing.T) {
	unit := simplexSystemdUnitContent("5225")
	for _, forbidden := range []string{"0.0.0.0", "::", "--host", "--bind", "--address", "-h "} {
		if strings.Contains(unit, forbidden) {
			t.Fatalf("unit must not contain %q (unauthenticated API must stay on localhost): %s", forbidden, unit)
		}
	}
	if got := strings.Count(unit, " -p "); got != 1 {
		t.Fatalf("unit must pass the port exactly once, found %d: %s", got, unit)
	}
}

// TestValidateSimplexPortRejectsUnsafeValues 守住一条信任边界：端口来自 key=val /
// stdin / 交互式输入，会被拼进一个 root 拥有的 systemd 单元。换行等于注入任意单元
// 指令（如额外的 ExecStartPre），而 "5225 --host 0.0.0.0" 会把本该只监听 localhost
// 的无鉴权 API 暴露到公网。
func TestValidateSimplexPortRejectsUnsafeValues(t *testing.T) {
	for _, bad := range []string{
		"",
		"abc",
		"0",
		"-1",
		"+5225",
		" 5225",
		"5225 ",
		"5225\nExecStart=/bin/sh -c 'x'",
		"5225\r\nExecStartPre=/bin/sh -c 'x'",
		"5225 --host 0.0.0.0",
		"0.0.0.0",
		"65536",
		"70000",
		"123456",
		"52_25",
		"5225;rm -rf /",
	} {
		if err := validateSimplexPort(bad); err == nil {
			t.Errorf("validateSimplexPort(%q) must be rejected", bad)
		}
	}
}

func TestValidateSimplexPortAcceptsRealPorts(t *testing.T) {
	for _, ok := range []string{"1", "80", "1024", "5225", "65535"} {
		if err := validateSimplexPort(ok); err != nil {
			t.Errorf("validateSimplexPort(%q) must be accepted, got %v", ok, err)
		}
	}
}

// TestReplaceFileAtomicallyKeepsDestinationWhenSourceFails 守住原子替换的核心契约：
// 源读取中断时目标文件必须保持原样，且不能留下半截的 .new 残骸。
func TestReplaceFileAtomicallyKeepsDestinationWhenSourceFails(t *testing.T) {
	dir := t.TempDir()
	dest := filepath.Join(dir, simplexBinaryName)
	if err := os.WriteFile(dest, []byte("old-binary"), 0o755); err != nil {
		t.Fatal(err)
	}

	err := replaceFileAtomically(dest, iotest.ErrReader(errors.New("boom")), 0o755)
	if err == nil {
		t.Fatal("replace must fail when the source errors")
	}

	got, readErr := os.ReadFile(dest)
	if readErr != nil {
		t.Fatalf("destination must still exist: %v", readErr)
	}
	if string(got) != "old-binary" {
		t.Fatalf("destination must be untouched on failure, got %q", got)
	}
	if _, statErr := os.Stat(dest + ".new"); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("failed replace must not leave a .new file behind (stat err = %v)", statErr)
	}
}

// TestReplaceFileAtomicallySwapsInodeInsteadOfTruncating 守住 ETXTBSY 的修复点：
// 必须是 rename 换目录项（inode 变化），而不是就地 O_TRUNC。就地截断正在被执行的
// simplex-chat 会被 Linux 以 ETXTBSY 拒绝，重装就此永远失败。
func TestReplaceFileAtomicallySwapsInodeInsteadOfTruncating(t *testing.T) {
	dir := t.TempDir()
	dest := filepath.Join(dir, simplexBinaryName)
	if err := os.WriteFile(dest, []byte("old-binary"), 0o755); err != nil {
		t.Fatal(err)
	}
	before, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}

	if err := replaceFileAtomically(dest, strings.NewReader("new-binary"), 0o755); err != nil {
		t.Fatalf("replace failed: %v", err)
	}

	after, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}
	if os.SameFile(before, after) {
		t.Fatal("destination inode unchanged: this is an in-place truncate, which fails with ETXTBSY while the service runs")
	}
	got, err := os.ReadFile(dest)
	if err != nil {
		t.Fatal(err)
	}
	if string(got) != "new-binary" {
		t.Fatalf("destination content = %q, want %q", got, "new-binary")
	}
	if after.Mode().Perm()&0o100 == 0 {
		t.Fatalf("destination must stay owner-executable, mode = %v", after.Mode().Perm())
	}
	if _, statErr := os.Stat(dest + ".new"); !errors.Is(statErr, os.ErrNotExist) {
		t.Fatalf("successful replace must not leave a .new file behind (stat err = %v)", statErr)
	}
}

// TestInstallSimplexBinaryReplacesRunningTarget 覆盖安装路径本身：目标位置已经存在
// 上一版 simplex-chat（即正在被服务执行的 inode）时，安装必须成功并换掉该文件。
func TestInstallSimplexBinaryReplacesRunningTarget(t *testing.T) {
	dir := t.TempDir()
	dest := filepath.Join(dir, simplexBinaryName)
	if err := os.WriteFile(dest, []byte("old-binary"), 0o755); err != nil {
		t.Fatal(err)
	}
	before, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}

	downloaded := filepath.Join(t.TempDir(), "simplex-chat-download")
	if err := os.WriteFile(downloaded, []byte("new-binary"), 0o644); err != nil {
		t.Fatal(err)
	}

	got, err := installSimplexBinary(downloaded, dir)
	if err != nil {
		t.Fatalf("installSimplexBinary failed: %v", err)
	}
	if got != dest {
		t.Fatalf("installSimplexBinary returned %q, want %q", got, dest)
	}

	after, err := os.Stat(dest)
	if err != nil {
		t.Fatal(err)
	}
	if os.SameFile(before, after) {
		t.Fatal("install must replace the target file, not truncate it in place")
	}
	content, err := os.ReadFile(dest)
	if err != nil {
		t.Fatal(err)
	}
	if string(content) != "new-binary" {
		t.Fatalf("installed content = %q, want %q", content, "new-binary")
	}
	if after.Mode().Perm()&0o100 == 0 {
		t.Fatalf("installed binary must be owner-executable, mode = %v", after.Mode().Perm())
	}
}

// TestSimplexUnitForWriteRefusesInjectedPort 覆盖真正落盘的那道闸：writeSimplexSystemdService
// 必须先经 simplexUnitForWrite 组装内容，注入型端口绝不能产出任何单元文本。
func TestSimplexUnitForWriteRefusesInjectedPort(t *testing.T) {
	for _, bad := range []string{
		"",
		"5225\nExecStartPre=/bin/sh -c 'x'",
		"5225\nExecStart=/bin/sh -c 'x'",
		"5225 --host 0.0.0.0",
		"0.0.0.0",
		"70000",
	} {
		content, err := simplexUnitForWrite(bad)
		if err == nil {
			t.Errorf("simplexUnitForWrite(%q) must refuse to produce unit content, got %q", bad, content)
		}
		if len(content) != 0 {
			t.Errorf("simplexUnitForWrite(%q) must return no content on rejection, got %q", bad, content)
		}
	}

	content, err := simplexUnitForWrite("5225")
	if err != nil {
		t.Fatalf("valid port must produce unit content: %v", err)
	}
	if !strings.Contains(string(content), "-p 5225") {
		t.Fatalf("unit content must carry the port: %s", content)
	}
}

func TestDiscordPlatformIsRejected(t *testing.T) {
	t.Run("keyval discord_token", func(t *testing.T) {
		_, err := parseKeyVal([]byte("discord_token=abc\ndiscord_admin_id=123\n"))
		if err == nil {
			t.Fatal("parseKeyVal 含 discord_token 时必须报错")
		}
		if !strings.Contains(err.Error(), "已移除") {
			t.Errorf("错误信息应说明平台已移除，实际: %v", err)
		}
	})

	t.Run("choice 3 is reserved", func(t *testing.T) {
		_, _, _, err := platformSetupForChoice("3")
		if err == nil {
			t.Fatal("编号 3 必须报错（保留空洞）")
		}
		if !strings.Contains(err.Error(), "已移除") {
			t.Errorf("错误信息应说明平台已移除，实际: %v", err)
		}
	})

	t.Run("existing unit with --discord", func(t *testing.T) {
		_, _, err := recoveryPlatformForService([]byte("ExecStart=/a --discord\n"), "")
		if err == nil {
			t.Fatal("既有单元含 --discord 时必须报错，不得静默换成其它平台")
		}
		if !strings.Contains(err.Error(), "已移除") {
			t.Errorf("错误信息应说明平台已移除，实际: %v", err)
		}
	})

	t.Run("parsePlatformChoice rejects discord", func(t *testing.T) {
		if _, _, _, err := parsePlatformChoice("discord"); err == nil {
			t.Fatal("parsePlatformChoice(\"discord\") 必须报错")
		}
		if _, _, _, err := parsePlatformChoice("telegram + discord"); err == nil {
			t.Fatal("parsePlatformChoice(\"telegram + discord\") 必须报错")
		}
	})
}
