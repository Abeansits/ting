package main

import (
	"context"
	"os"
	"path/filepath"
	"testing"
	"time"

	"github.com/Abeansits/ting/tui/internal/data"
	"github.com/Abeansits/ting/tui/internal/model"
)

// TestFixtureReplay is the end-to-end data-spine test: snapshot + JSONL
// fixtures drive the reducer to a consistent final state.
func TestFixtureReplay(t *testing.T) {
	dir := filepath.Join("testdata", "sample-forum")

	state, err := data.LoadState(dir)
	if err != nil || state == nil {
		t.Fatalf("LoadState: state=%v err=%v", state, err)
	}

	tail, err := data.NewTailer(data.EventLogPath(dir))
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)

	// Fold every event the fixture contains (seq <= LatestSeq are skipped
	// by Apply; that's the whole point of the idempotency check).
	deadline := time.After(2 * time.Second)
	const wantEvents = 4
	seen := 0
loop:
	for seen < wantEvents {
		select {
		case ev, ok := <-tail.Events():
			if !ok {
				break loop
			}
			if err := state.Apply(ev); err != nil {
				t.Fatalf("Apply seq=%d: %v", ev.Seq, err)
			}
			seen++
		case <-deadline:
			t.Fatalf("only saw %d/%d events", seen, wantEvents)
		}
	}

	if state.Topic != "Is the sky falling?" {
		t.Errorf("Topic = %q", state.Topic)
	}
	if state.Status != model.StatusInProgress {
		t.Errorf("Status = %q, want in_progress", state.Status)
	}
	if state.LatestSeq != 4 {
		t.Errorf("LatestSeq = %d, want 4", state.LatestSeq)
	}
	if state.ClassifierMetrics == nil {
		t.Error("ClassifierMetrics not populated (snapshot provided them)")
	}
}

// TestReplayFixtureOnEmptyState verifies a cold start with no snapshot:
// events alone should drive the reducer to a coherent state.
func TestReplayFixtureOnEmptyState(t *testing.T) {
	dir := t.TempDir()
	path := data.EventLogPath(dir)
	body, err := os.ReadFile(filepath.Join("testdata", "sample-forum", "dashboard-events.jsonl"))
	if err != nil {
		t.Fatalf("read fixture: %v", err)
	}
	if err := os.WriteFile(path, body, 0o644); err != nil {
		t.Fatal(err)
	}

	tail, err := data.NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)

	state := model.NewState("")
	deadline := time.After(2 * time.Second)
	for state.LatestSeq < 4 {
		select {
		case ev, ok := <-tail.Events():
			if !ok {
				t.Fatalf("events channel closed at seq=%d", state.LatestSeq)
			}
			if err := state.Apply(ev); err != nil {
				t.Fatalf("Apply seq=%d: %v", ev.Seq, err)
			}
		case <-deadline:
			t.Fatalf("stuck at LatestSeq=%d", state.LatestSeq)
		}
	}

	if state.Topic != "Is the sky falling?" || state.MaxRounds != 3 {
		t.Errorf("forum_started did not seed state: %+v", state)
	}
	if len(state.Rounds) != 1 || state.Rounds[0].Stage != "proposal" {
		t.Errorf("rounds not built: %+v", state.Rounds)
	}
	if len(state.Rounds[0].ParticipantsResponded) != 1 {
		t.Errorf("ParticipantsResponded = %v, want [codex]", state.Rounds[0].ParticipantsResponded)
	}
}
