// Package data reads dashboard-state.json and tails dashboard-events.jsonl.
// The TUI talks to the filesystem directly; it does not depend on a running
// Rust server.
package data

import (
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"path/filepath"

	"github.com/Abeansits/ting/tui/internal/model"
)

// Must stay in sync with src/events.rs and src/dashboard_state.rs.
const (
	StateFilename    = "dashboard-state.json"
	EventLogFilename = "dashboard-events.jsonl"
)

func StatePath(forumDir string) string    { return filepath.Join(forumDir, StateFilename) }
func EventLogPath(forumDir string) string { return filepath.Join(forumDir, EventLogFilename) }

// LoadState reads dashboard-state.json from forumDir. Returns (nil, nil) if
// the snapshot is missing — consumers replay the event log from seq=1.
func LoadState(forumDir string) (*model.State, error) {
	path := StatePath(forumDir)
	body, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return nil, nil
		}
		return nil, err
	}
	var s model.State
	if err := json.Unmarshal(body, &s); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	return &s, nil
}
