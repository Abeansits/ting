package data

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"
)

const sampleEvent = `{"version":1,"seq":%d,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"round_started","payload":{"round":%d,"stage":"proposal"}}`

func newEmptyLog(t *testing.T) (dir, path string) {
	t.Helper()
	dir = t.TempDir()
	path = EventLogPath(dir)
	// Create empty file so the watcher can seed; the tailer also handles the
	// missing-file case, but having the file present exercises the drain-
	// first branch of Run.
	f, err := os.Create(path)
	if err != nil {
		t.Fatalf("create %s: %v", path, err)
	}
	_ = f.Close()
	return dir, path
}

func TestTailer_DrainExistingLines(t *testing.T) {
	dir := t.TempDir()
	path := EventLogPath(dir)

	// Seed two events before the tailer starts.
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)

	seqs := collect(t, tail, 2, 2*time.Second)
	if seqs[0] != 1 || seqs[1] != 2 {
		t.Errorf("seqs = %v, want [1 2]", seqs)
	}
}

func TestTailer_EmitsOnAppend(t *testing.T) {
	dir, path := newEmptyLog(t)
	_ = dir

	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)

	// Give the tailer a beat to finish the initial drain before we append.
	time.Sleep(50 * time.Millisecond)

	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	seqs := collect(t, tail, 2, 2*time.Second)
	if seqs[0] != 1 || seqs[1] != 2 {
		t.Errorf("seqs = %v, want [1 2]", seqs)
	}
}

func TestTailer_PartialLineHeldUntilNewline(t *testing.T) {
	dir, path := newEmptyLog(t)
	_ = dir

	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)
	time.Sleep(50 * time.Millisecond)

	// Write the first half WITHOUT a trailing newline.
	evt := fmt.Sprintf(sampleEvent, 1, 1)
	half := evt[:len(evt)/2]
	rest := evt[len(evt)/2:]

	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0o644)
	if err != nil {
		t.Fatal(err)
	}
	if _, err := f.WriteString(half); err != nil {
		t.Fatal(err)
	}
	_ = f.Close()

	// Should NOT emit yet — no newline.
	select {
	case ev := <-tail.Events():
		t.Fatalf("tailer emitted partial line: %+v", ev)
	case <-time.After(150 * time.Millisecond):
	}

	// Complete the line.
	if err := appendLine(path, rest); err != nil {
		t.Fatal(err)
	}

	seqs := collect(t, tail, 1, 2*time.Second)
	if seqs[0] != 1 {
		t.Errorf("seqs = %v, want [1]", seqs)
	}
}

func TestTailer_SkipsMalformedLines(t *testing.T) {
	dir, path := newEmptyLog(t)
	_ = dir

	// Mix a valid, a malformed, and another valid line all before Run.
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, "not json at all"); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)

	seqs := collect(t, tail, 2, 2*time.Second)
	if seqs[0] != 1 || seqs[1] != 2 {
		t.Errorf("seqs = %v, want [1 2] (malformed should be skipped)", seqs)
	}
}

func TestTailer_SkipsUnknownEventType(t *testing.T) {
	dir, path := newEmptyLog(t)
	_ = dir

	if err := appendLine(path,
		`{"version":1,"seq":1,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"brand_new","payload":{}}`); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)

	// Only the known-type event should be emitted.
	seqs := collect(t, tail, 1, 2*time.Second)
	if seqs[0] != 2 {
		t.Errorf("seqs = %v, want [2]", seqs)
	}
}

func TestTailer_MissingFileThenCreate(t *testing.T) {
	dir := t.TempDir()
	path := EventLogPath(dir)
	// No file yet; tailer should still start.

	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go tail.Run(ctx)
	time.Sleep(50 * time.Millisecond)

	// Create the file and append an event.
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}

	seqs := collect(t, tail, 1, 2*time.Second)
	if seqs[0] != 1 {
		t.Errorf("seqs = %v, want [1]", seqs)
	}
}

func TestEventLogPath(t *testing.T) {
	got := EventLogPath("/tmp/forum")
	want := filepath.Join("/tmp/forum", "dashboard-events.jsonl")
	if got != want {
		t.Errorf("EventLogPath = %q, want %q", got, want)
	}
}

// collect drains n events from the tailer or fails the test on timeout.
func collect(t *testing.T, tail *Tailer, n int, timeout time.Duration) []uint64 {
	t.Helper()
	seqs := make([]uint64, 0, n)
	deadline := time.After(timeout)
	for len(seqs) < n {
		select {
		case ev, ok := <-tail.Events():
			if !ok {
				t.Fatalf("tailer events closed; got %d/%d", len(seqs), n)
			}
			seqs = append(seqs, ev.Seq)
		case <-deadline:
			t.Fatalf("timed out; got %d/%d seqs=%v", len(seqs), n, seqs)
		}
	}
	return seqs
}
