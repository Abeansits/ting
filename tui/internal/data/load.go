// Package data handles the filesystem-substrate side of the TUI: reading
// dashboard-state.json for the initial frame and tailing
// dashboard-events.jsonl for live updates. The Go TUI talks to the
// filesystem directly; it does not depend on a running Rust server. See the
// "filesystem-first" dissent in .coord/plan-v2.md for the rationale.
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

// Filenames produced by the Rust writer inside a forum directory. Must stay
// in sync with src/events.rs (EVENT_LOG_FILENAME) and src/dashboard_state.rs
// (STATE_FILENAME).
const (
	StateFilename    = "dashboard-state.json"
	EventLogFilename = "dashboard-events.jsonl"
)

func StatePath(forumDir string) string    { return filepath.Join(forumDir, StateFilename) }
func EventLogPath(forumDir string) string { return filepath.Join(forumDir, EventLogFilename) }

// LoadState reads dashboard-state.json from forumDir. Returns (nil, nil) if
// the snapshot is missing — consumers are expected to start from an empty
// state and replay the event log.
func LoadState(forumDir string) (*model.State, error) {
	path := StatePath(forumDir)
	body, err := os.ReadFile(path)
	if err != nil {
		if errors.Is(err, fs.ErrNotExist) {
			return nil, nil
		}
		return nil, fmt.Errorf("read %s: %w", path, err)
	}
	var s model.State
	if err := json.Unmarshal(body, &s); err != nil {
		return nil, fmt.Errorf("parse %s: %w", path, err)
	}
	return &s, nil
}
