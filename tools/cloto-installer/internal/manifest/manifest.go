// Package manifest reads what a connector says about itself, from the
// connector's own `cloto-connector.json` rather than from the catalog.
//
// The engine reads it for one question: is there a server here to build? The
// catalog cannot answer that. Its wire shape carries no `connector_type`, so
// the declaration is dropped exactly at the boundary the engine would read it
// from — the same reason `managers::connector_manifest` in the kernel reads
// the file and not the catalog. The engine has the extracted tree in hand
// before it builds anything, which is the first moment the question can be
// asked and the last one at which the answer still changes what it does.
//
// It deliberately does not decide whether the host *supports* what it reads.
// That is the kernel's call, made against the kernel's own version, and a
// second copy of it here would be a second answer to a settled question.
package manifest

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
)

// FileName is the manifest a connector ships beside its files.
const FileName = "cloto-connector.json"

// DefaultConnectorType is what a connector is assumed to be when it does not
// say. Every connector predating the manifest is one of these, so absent has
// to mean the type that existed when they were written — otherwise reading
// the declaration would be a breaking change dressed as a check.
const DefaultConnectorType = "mgp_server"

// Declaration is the manifest reduced to what the engine acts on.
type Declaration struct {
	// ConnectorType as declared, or DefaultConnectorType when absent.
	ConnectorType string
	// Declared distinguishes "said mgp_server" from "said nothing". The
	// engine treats them alike today; a caller that stops doing so should
	// not have to reread the file to find out.
	Declared bool
	// PanelEntries are the distinct `ui.panels[].entry` values, in
	// declaration order. Empty when the connector declares no panels.
	PanelEntries []string
}

type wire struct {
	ConnectorType *string `json:"connector_type"`
	UI            *struct {
		Panels []struct {
			Entry string `json:"entry"`
		} `json:"panels"`
	} `json:"ui"`
}

// ErrNoEntry reports that a connector ships no server and names no single
// file for the integrity check to hash.
var ErrNoEntry = errors.New("no unambiguous entry point")

// Read returns what the connector in dir declares.
//
// A missing or unreadable manifest is not an error: it is the shape every
// connector had before manifests existed, and it reads as the default.
func Read(dir string) Declaration {
	d := Declaration{ConnectorType: DefaultConnectorType}
	raw, err := os.ReadFile(filepath.Join(dir, FileName))
	if err != nil {
		return d
	}
	var w wire
	if err := json.Unmarshal(raw, &w); err != nil {
		return d
	}
	d.Declared = true
	if w.ConnectorType != nil && strings.TrimSpace(*w.ConnectorType) != "" {
		d.ConnectorType = strings.TrimSpace(*w.ConnectorType)
	}
	if w.UI != nil {
		seen := map[string]bool{}
		for _, p := range w.UI.Panels {
			e := strings.TrimSpace(p.Entry)
			if e == "" || seen[e] {
				continue
			}
			seen[e] = true
			d.PanelEntries = append(d.PanelEntries, e)
		}
	}
	return d
}

// BuildsAServer reports whether the engine has something to build, given the
// connector types the host starts a process for.
//
// The list comes from the host rather than from a constant here, because the
// host owns it: a kernel that learns a new launchable type would otherwise
// have to be shipped alongside an engine taught the same thing separately.
// An empty list means the host said nothing, and the engine falls back to the
// one type that existed before any of this — never to "build everything",
// which is the reading that made a panel a Python project.
func (d Declaration) BuildsAServer(launchable []string) bool {
	if len(launchable) == 0 {
		launchable = []string{DefaultConnectorType}
	}
	for _, t := range launchable {
		if d.ConnectorType == t {
			return true
		}
	}
	return false
}

// EntryPoint names the file the integrity check should hash for a connector
// that builds no server, relative to the connector's directory.
//
// The catalog carries the hash of a file it does not name, so somebody has to
// decide which file that was. For a server the naming convention answers it
// (`server.py`), and the convention is right because those connectors are why
// it exists. For anything else the convention is a guess, and a guess that
// happens to be wrong is how a panel came to be installed as a Python
// project — so this refuses instead, and says what it could not decide.
func (d Declaration) EntryPoint() (string, error) {
	switch len(d.PanelEntries) {
	case 1:
		return d.PanelEntries[0], nil
	case 0:
		return "", ErrNoEntry
	default:
		return "", ErrNoEntry
	}
}
