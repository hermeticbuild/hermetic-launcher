package main

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
)

func main() {
	fmt.Println("Hello from Go!")
	if len(os.Args) < 4 {
		fmt.Fprintf(os.Stderr, "usage: %s <py_binary> <cc_binary> <sh_binary>\n", os.Args[0])
		os.Exit(1)
	}

	expected := []struct {
		path   string
		needle string
	}{
		{os.Args[1], "Hello from Python!"},
		{os.Args[2], "Hello from C++!"},
		{os.Args[3], "Hello from Shell!"},
	}

	for _, e := range expected {
		out, err := exec.Command(e.path).Output()
		if err != nil {
			fmt.Fprintf(os.Stderr, "failed to run %s: %v\n", e.path, err)
			os.Exit(1)
		}
		if !strings.Contains(string(out), e.needle) {
			fmt.Fprintf(os.Stderr, "expected %q in output of %s, got: %s\n", e.needle, e.path, out)
			os.Exit(1)
		}
		fmt.Print(string(out))
	}

	fmt.Println("PASS: all four languages produced output")
}
