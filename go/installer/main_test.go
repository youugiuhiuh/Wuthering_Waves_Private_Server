package main

import (
	"bytes"
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
