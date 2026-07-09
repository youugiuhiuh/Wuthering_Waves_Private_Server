package main

import (
	"bytes"
	"encoding/json"
	"testing"
)

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
			"", "", "", nil, nil, "", "",
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
			"https://matrix.org", "@bot:matrix.org", "!room:matrix.org", []byte("pass123"), nil, "", "",
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
			"https://matrix.org", "", "", nil, nil, "", "",
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

	t.Run("with discord fields", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("t"), []byte("1"), []byte("S"),
			"", "", "", nil, nil,
			"MTIzLmFiYw", "123456789",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("invalid JSON: %v", err)
		}
		if parsed["discord_token"] != "MTIzLmFiYw" {
			t.Errorf("discord_token = %v, want MTIzLmFiYw", parsed["discord_token"])
		}
		if parsed["discord_admin_id"] != "123456789" {
			t.Errorf("discord_admin_id = %v, want 123456789", parsed["discord_admin_id"])
		}
	})

	t.Run("without discord fields", func(t *testing.T) {
		payload := buildSetupPayload(
			[]byte("t"), []byte("1"), []byte("S"),
			"", "", "", nil, nil, "", "",
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
			cfg.MatrixHS, cfg.MatrixUser, cfg.MatrixRoom, []byte(cfg.MatrixPassword), nil, "", "",
		)
		var parsed map[string]interface{}
		if err := json.Unmarshal(payload, &parsed); err != nil {
			t.Fatalf("payload should be valid JSON: %v", err)
		}
	})

	t.Run("with discord fields", func(t *testing.T) {
		data := []byte("token=t\nadmin_id=1\ndiscord_token=MTIzLmFiYw\ndiscord_admin_id=123456789\n")
		cfg, err := parseKeyVal(data)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if cfg.DiscordToken != "MTIzLmFiYw" {
			t.Errorf("DiscordToken = %q, want MTIzLmFiYw", cfg.DiscordToken)
		}
		if cfg.DiscordAdminID != "123456789" {
			t.Errorf("DiscordAdminID = %q, want 123456789", cfg.DiscordAdminID)
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
