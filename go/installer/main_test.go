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
			"", "", "", nil,
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
			"https://matrix.org", "@bot:matrix.org", "!room:matrix.org", []byte("pass123"),
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
			"https://matrix.org", "", "", nil,
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
}
