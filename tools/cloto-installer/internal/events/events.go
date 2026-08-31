// Package events writes the installer's progress stream: one JSON object
// per line on stdout, in the same shape the kernel's setup progress events
// have on its SSE feed (`{"type":"StepStart","step":...}`), so the kernel
// can forward each line as-is. The last line of a stage is a `Result`.
package events

import (
	"encoding/json"
	"io"
	"sync"
)

// Emitter serialises events to one writer. It is safe for concurrent use:
// a child process's stderr is streamed from a separate goroutine.
type Emitter struct {
	mu sync.Mutex
	w  io.Writer
}

// New returns an emitter writing to w.
func New(w io.Writer) *Emitter {
	return &Emitter{w: w}
}

func (e *Emitter) write(v any) {
	data, err := json.Marshal(v)
	if err != nil {
		// Every event type here marshals; a failure is a programming error.
		panic(err)
	}
	e.mu.Lock()
	defer e.mu.Unlock()
	_, _ = e.w.Write(append(data, '\n'))
}

// StepStart marks the beginning of a step.
func (e *Emitter) StepStart(step, description string) {
	e.write(struct {
		Type        string `json:"type"`
		Step        string `json:"step"`
		Description string `json:"description"`
	}{"StepStart", step, description})
}

// StepProgress reports progress within a step; progress is -1 when the
// step has no measurable fraction.
func (e *Emitter) StepProgress(step string, progress float32, detail string) {
	e.write(struct {
		Type     string  `json:"type"`
		Step     string  `json:"step"`
		Progress float32 `json:"progress"`
		Detail   string  `json:"detail"`
	}{"StepProgress", step, progress, detail})
}

// StepComplete marks a step as finished.
func (e *Emitter) StepComplete(step string) {
	e.write(struct {
		Type string `json:"type"`
		Step string `json:"step"`
	}{"StepComplete", step})
}

// StepError reports why a step stopped the install. `recoverable` keeps the
// distinction the kernel draws between a retryable condition (network,
// dependency resolution) and one that must not be retried blindly
// (tampering, a malformed catalog entry).
func (e *Emitter) StepError(step, errText string, recoverable bool) {
	e.write(struct {
		Type        string `json:"type"`
		Step        string `json:"step"`
		Error       string `json:"error"`
		Recoverable bool   `json:"recoverable"`
	}{"StepError", step, errText, recoverable})
}

// ServerInstall reports a per-server status change (`installing`,
// `installed`).
func (e *Emitter) ServerInstall(serverName, status string) {
	e.write(struct {
		Type       string `json:"type"`
		ServerName string `json:"server_name"`
		Status     string `json:"status"`
	}{"ServerInstall", serverName, status})
}

// Result writes the stage's final line. `v` must marshal to an object; the
// emitter adds `"type":"Result"` by wrapping it.
func (e *Emitter) Result(v any) {
	data, err := json.Marshal(v)
	if err != nil {
		panic(err)
	}
	var fields map[string]any
	if err := json.Unmarshal(data, &fields); err != nil {
		panic(err)
	}
	fields["type"] = "Result"
	e.write(fields)
}
