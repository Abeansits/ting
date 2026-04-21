// Package app wires the Bubble Tea model: load the snapshot, tail the
// JSONL, fold events into State. The view layer is a placeholder.
package app

import (
	"context"

	"github.com/Abeansits/ting/tui/internal/data"
	"github.com/Abeansits/ting/tui/internal/model"
)

type Model struct {
	state   *model.State
	tailer  *data.Tailer
	cancel  context.CancelFunc
	ctx     context.Context
	loadErr error
	tailErr error
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
		state:   state,
		tailer:  tailer,
		cancel:  cancel,
		ctx:     ctx,
		loadErr: loadErr,
	}, nil
}

// Close stops the tailer. Safe to call twice.
func (m *Model) Close() {
	m.cancel()
	_ = m.tailer.Close()
}

// State returns the current reduced state. Intended for tests.
func (m *Model) State() *model.State { return m.state }
