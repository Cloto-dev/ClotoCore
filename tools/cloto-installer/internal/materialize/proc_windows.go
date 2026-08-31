//go:build windows

package materialize

import (
	"os/exec"
	"syscall"
)

// hideWindow keeps a child console process from flashing a window on
// Windows (CREATE_NO_WINDOW).
func hideWindow(cmd *exec.Cmd) {
	cmd.SysProcAttr = &syscall.SysProcAttr{CreationFlags: 0x08000000}
}
