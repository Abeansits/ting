// Package app wires the Bubble Tea model: load the snapshot, tail the
// JSONL, fold events into State, and render the TUI.
package app

import (
	"context"
	"time"

	"github.com/Abeansits/ting/tui/internal/data"
	"github.com/Abeansits/ting/tui/internal/model"
)

type Model struct {
	state    *model.State
	tailer   *data.Tailer
	cancel   context.CancelFunc
	ctx      context.Context
	forumDir string
	loadErr  error
	tailErr  error

	now          time.Time
	focusedRound int
	showHelp     bool
	ticking      bool
	cache        viewCache
}

// New reads the snapshot (if any) and opens the tailer watcher. The tailer
// goroutine is not started until Init. Call Close to release the watcher if
// the program exits before Init.
func New(parent context.Context, forumDir string) (*Model, error) {
	state, loadErr := data.LoadState(forumDir)
	if state == nil {
		state = model.NewState("")
	}

	tailer, err := data.NewTailer(data.EventLogPath(forumDir))
	if err != nil {
		return nil, err
	}

	ctx, cancel := context.WithCancel(parent)
	return &Model{
		state:    state,
		tailer:   tailer,
		cancel:   cancel,
		ctx:      ctx,
		forumDir: forumDir,
		loadErr:  loadErr,
		now:      time.Now(),
	}, nil
}

// Close stops the tailer. Safe to call twice.
func (m *Model) Close() {
	m.cancel()
	_ = m.tailer.Close()
}

// State returns the current reduced state. Intended for tests.
func (m *Model) State() *model.State { return m.state }

// animating returns true while the spinner needs to advance — i.e. the forum
// is still in progress. Completed/pending forums hold the frame and stop the
// tick loop, saving per-tick CPU when the dashboard is parked open.
func (m *Model) animating() bool { return m.state.Status == model.StatusInProgress }
