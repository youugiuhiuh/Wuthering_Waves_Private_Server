package isolate

import (
	"encoding/json"
	"os"
	"time"
)

type State struct {
	Namespace  string    `json:"namespace"`
	WiFiIface  string    `json:"wifi_iface"`
	Status     string    `json:"status"`
	PID        int       `json:"pid"`
	NMManaged  bool      `json:"nm_managed"`
	StartedAt  time.Time `json:"started_at"`
}

func LoadState(path string) (*State, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var s State
	if err := json.Unmarshal(data, &s); err != nil {
		return nil, err
	}
	return &s, nil
}

func SaveState(path string, s *State) error {
	data, err := json.MarshalIndent(s, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(path, data, 0600)
}

func DeleteState(path string) error {
	if err := os.Remove(path); err != nil && !os.IsNotExist(err) {
		return err
	}
	return nil
}
