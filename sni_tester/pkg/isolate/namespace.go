package isolate

import (
	"fmt"
	"os/exec"
)

func CreateNamespace(name string) error {
	return run("ip", "netns", "add", name)
}

func DeleteNamespace(name string) error {
	if !NamespaceExists(name) {
		return nil
	}
	return run("ip", "netns", "delete", name)
}

func NamespaceExists(name string) bool {
	return exec.Command("ip", "netns", "pids", name).Run() == nil
}

func ExecInNamespace(ns string, args ...string) ([]byte, error) {
	full := append([]string{"netns", "exec", ns}, args...)
	cmd := exec.Command("ip", full...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return nil, fmt.Errorf("%v: %s", err, string(out))
	}
	return out, nil
}

func run(name string, args ...string) error {
	cmd := exec.Command(name, args...)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("%s %v: %s", name, args, string(out))
	}
	return nil
}
