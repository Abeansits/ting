package data

import (
	"context"
	"fmt"
	"os"
	"path/filepath"
	"testing"
	"time"
)

const (
	sampleEvent = `{"version":1,"seq":%d,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"round_started","payload":{"round":%d,"stage":"proposal"}}`
	// fsnotify has OS-dependent delivery latency; 2s is generous enough to
	// tolerate slow CI without masking real stalls.
	testTimeout = 2 * time.Second
)

func newEmptyLog(t *testing.T) string {
	t.Helper()
	path := EventLogPath(t.TempDir())
	f, err := os.Create(path)
	if err != nil {
		t.Fatalf("create %s: %v", path, err)
	}
	_ = f.Close()
	return path
}

func startTailer(t *testing.T, path string) (*Tailer, context.CancelFunc) {
	t.Helper()
	tail, err := NewTailer(path)
	if err != nil {
		t.Fatalf("NewTailer: %v", err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	go tail.Run(ctx)
	// Block until the initial drain finishes so the test is synced with
	// the tailer before appending.
	select {
	case <-tail.Ready():
	case <-time.After(testTimeout):
		cancel()
		t.Fatalf("tailer never became ready")
	}
	return tail, cancel
}

func TestTailer_DrainExistingLines(t *testing.T) {
	path := EventLogPath(t.TempDir())
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	tail, cancel := startTailer(t, path)
	defer cancel()

	seqs := collect(t, tail, 2)
	if seqs[0] != 1 || seqs[1] != 2 {
		t.Errorf("seqs = %v, want [1 2]", seqs)
	}
}

func TestTailer_EmitsOnAppend(t *testing.T) {
	path := newEmptyLog(t)
	tail, cancel := startTailer(t, path)
	defer cancel()

	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	seqs := collect(t, tail, 2)
	if seqs[0] != 1 || seqs[1] != 2 {
		t.Errorf("seqs = %v, want [1 2]", seqs)
	}
}

func TestTailer_PartialLineHeldUntilNewline(t *testing.T) {
	path := newEmptyLog(t)
	tail, cancel := startTailer(t, path)
	defer cancel()

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

	// No emit expected yet — no terminating newline.
	select {
	case ev := <-tail.Events():
		t.Fatalf("tailer emitted partial line: %+v", ev)
	case <-time.After(150 * time.Millisecond):
	}

	if err := appendLine(path, rest); err != nil {
		t.Fatal(err)
	}

	seqs := collect(t, tail, 1)
	if seqs[0] != 1 {
		t.Errorf("seqs = %v, want [1]", seqs)
	}
}

func TestTailer_SkipsMalformedLines(t *testing.T) {
	path := newEmptyLog(t)
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, "not json at all"); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	tail, cancel := startTailer(t, path)
	defer cancel()

	seqs := collect(t, tail, 2)
	if seqs[0] != 1 || seqs[1] != 2 {
		t.Errorf("seqs = %v, want [1 2] (malformed should be skipped)", seqs)
	}
}

func TestTailer_SkipsUnknownEventType(t *testing.T) {
	path := newEmptyLog(t)
	if err := appendLine(path,
		`{"version":1,"seq":1,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"brand_new","payload":{}}`); err != nil {
		t.Fatal(err)
	}
	if err := appendLine(path, fmt.Sprintf(sampleEvent, 2, 2)); err != nil {
		t.Fatal(err)
	}

	tail, cancel := startTailer(t, path)
	defer cancel()

	seqs := collect(t, tail, 1)
	if seqs[0] != 2 {
		t.Errorf("seqs = %v, want [2]", seqs)
	}
}

func TestTailer_MissingFileThenCreate(t *testing.T) {
	dir := t.TempDir()
	path := EventLogPath(dir)
	// No file yet; tailer should still come up (parent-dir watch).

	tail, cancel := startTailer(t, path)
	defer cancel()

	if err := appendLine(path, fmt.Sprintf(sampleEvent, 1, 1)); err != nil {
		t.Fatal(err)
	}

	seqs := collect(t, tail, 1)
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

func collect(t *testing.T, tail *Tailer, n int) []uint64 {
	t.Helper()
	seqs := make([]uint64, 0, n)
	deadline := time.After(testTimeout)
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
