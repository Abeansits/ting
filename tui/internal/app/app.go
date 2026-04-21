// Package app wires the Bubble Tea model together. Phase 3A ships the data
// spine — load the snapshot, tail the JSONL, fold events into State — with a
// placeholder view. Phase 3B fills in the real rendering (metrics bars,
// convergence gauge, round table, Dissent Axis) without touching the data
// layer.
package app

import (
	"context"
	"fmt"

	"github.com/Abeansits/ting/tui/internal/data"
	"github.com/Abeansits/ting/tui/internal/model"
)

// Model is the Bubble Tea model. It owns the current reduced State plus the
// tailer that feeds it. Phase 3B will attach styled sub-views; nothing in
// that layer needs to reach into the tailer directly.
type Model struct {
	forumDir string
	state    *model.State
	tailer   *data.Tailer
	ctx      context.Context
	cancel   context.CancelFunc
	loadErr  error
	tailErr  error
	closed   bool
}

// New prepares the model. It reads the snapshot (if any) and opens the
// tailer watcher but does NOT start the tailer goroutine — that happens
// from Init so Bubble Tea owns the lifecycle. Call Close to release the
// watcher if the program exits before Init (e.g. flag parse error upstream).
func New(parent context.Context, forumDir string) (*Model, error) {
	state, loadErr := data.LoadState(forumDir)
	if state == nil {
		state = model.NewState("")
	}

	tailer, err := data.NewTailer(data.EventLogPath(forumDir))
	if err != nil {
		return nil, fmt.Errorf("open tailer: %w", err)
	}

	ctx, cancel := context.WithCancel(parent)
	return &Model{
		forumDir: forumDir,
		state:    state,
		tailer:   tailer,
		ctx:      ctx,
		cancel:   cancel,
		loadErr:  loadErr,
	}, nil
}

// Close stops the tailer. Safe to call multiple times.
func (m *Model) Close() {
	if m.closed {
		return
	}
	m.closed = true
	m.cancel()
	_ = m.tailer.Close()
}

// State returns a pointer to the current reduced state. Primarily for tests;
// the Bubble Tea view should go through the Model.
func (m *Model) State() *model.State { return m.state }
