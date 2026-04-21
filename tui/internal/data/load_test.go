package data

import (
	"path/filepath"
	"testing"

	"github.com/Abeansits/ting/tui/internal/model"
)

func TestLoadState_Fixture(t *testing.T) {
	dir := filepath.Join("..", "..", "testdata", "sample-forum")
	s, err := LoadState(dir)
	if err != nil {
		t.Fatalf("LoadState: %v", err)
	}
	if s == nil {
		t.Fatal("LoadState returned nil on present fixture")
	}
	if s.ForumID != "ting-2026-04-19-abcd1234" {
		t.Errorf("ForumID = %q", s.ForumID)
	}
	if s.Status != model.StatusInProgress {
		t.Errorf("Status = %q, want in_progress", s.Status)
	}
	if s.LatestSeq != 4 {
		t.Errorf("LatestSeq = %d, want 4", s.LatestSeq)
	}
	if len(s.Rounds) != 1 || s.Rounds[0].Stage != "proposal" {
		t.Errorf("Rounds unexpected: %+v", s.Rounds)
	}
}

func TestLoadState_MissingReturnsNilNil(t *testing.T) {
	s, err := LoadState(t.TempDir())
	if err != nil {
		t.Fatalf("expected nil err for missing snapshot, got %v", err)
	}
	if s != nil {
		t.Errorf("expected nil state for missing snapshot, got %+v", s)
	}
}

func TestLoadState_MalformedReturnsError(t *testing.T) {
	dir := t.TempDir()
	path := StatePath(dir)
	if err := writeFile(path, []byte("not json")); err != nil {
		t.Fatal(err)
	}
	if _, err := LoadState(dir); err == nil {
		t.Fatal("expected parse error on malformed state.json")
	}
}
