package main

import "testing"

func TestNewConfigProvidesDefaultNetwork(t *testing.T) {
	cfg, err := newConfig("/tmp/sni_output")
	if err != nil {
		t.Fatalf("newConfig returned error: %v", err)
	}
	if cfg.Network == nil || cfg.Network.Dialer() == nil {
		t.Fatal("expected default network")
	}
}
