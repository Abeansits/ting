package model

import (
	"encoding/json"
	"testing"
)

func mustEvent(t *testing.T, seq uint64, eventType EventType, payload string) Event {
	t.Helper()
	return Event{
		Version: 1,
		Seq:     seq,
		ForumID: "ting-2026-04-19-abcd1234",
		Type:    eventType,
		Payload: json.RawMessage(payload),
	}
}

func TestApply_ForumStartedSeedsTopLevel(t *testing.T) {
	s := NewState("")
	err := s.Apply(mustEvent(t, 1, EventTypeForumStarted,
		`{"topic":"T","participants":["a","b"],"max_rounds":3}`))
	if err != nil {
		t.Fatal(err)
	}
	if s.Topic != "T" || len(s.Participants) != 2 || s.MaxRounds != 3 {
		t.Errorf("forum_started not applied: %+v", s)
	}
	if s.Status != StatusInProgress {
		t.Errorf("Status = %q, want in_progress", s.Status)
	}
	if s.LatestSeq != 1 {
		t.Errorf("LatestSeq = %d, want 1", s.LatestSeq)
	}
}

func TestApply_RoundAndParticipantResponse(t *testing.T) {
	s := NewState("")
	_ = s.Apply(mustEvent(t, 1, EventTypeRoundStarted, `{"round":1,"stage":"proposal"}`))
	_ = s.Apply(mustEvent(t, 2, EventTypeParticipantResponse, `{"round":1,"participant":"codex"}`))
	_ = s.Apply(mustEvent(t, 3, EventTypeParticipantResponse, `{"round":1,"participant":"gemini"}`))
	if len(s.Rounds) != 1 {
		t.Fatalf("len(Rounds) = %d, want 1", len(s.Rounds))
	}
	r := s.Rounds[0]
	if r.Stage != "proposal" {
		t.Errorf("Stage = %q, want proposal", r.Stage)
	}
	if len(r.ParticipantsResponded) != 2 {
		t.Errorf("ParticipantsResponded = %v", r.ParticipantsResponded)
	}
}

func TestApply_ConvergenceSetsRoundAndTopLevel(t *testing.T) {
	s := NewState("")
	_ = s.Apply(mustEvent(t, 1, EventTypeRoundStarted, `{"round":2,"stage":"synthesis"}`))
	_ = s.Apply(mustEvent(t, 2, EventTypeConvergence, `{"round":2,"score":6.1}`))
	if s.ConvergenceScore == nil || *s.ConvergenceScore != 6.1 {
		t.Errorf("top-level ConvergenceScore = %v, want 6.1", s.ConvergenceScore)
	}
	if s.Rounds[0].ConvergenceScore == nil || *s.Rounds[0].ConvergenceScore != 6.1 {
		t.Errorf("round ConvergenceScore = %v, want 6.1", s.Rounds[0].ConvergenceScore)
	}
}

func TestApply_ForumComplete(t *testing.T) {
	s := NewState("")
	_ = s.Apply(mustEvent(t, 1, EventTypeForumComplete, `{"rounds_used":2}`))
	if s.Status != StatusCompleted {
		t.Errorf("Status = %q, want completed", s.Status)
	}
}

func TestApply_IdempotentOnLowerSeq(t *testing.T) {
	s := NewState("")
	s.LatestSeq = 10
	// A seq <= LatestSeq event should be skipped without mutating fields.
	_ = s.Apply(mustEvent(t, 5, EventTypeForumComplete, `{"rounds_used":2}`))
	if s.Status == StatusCompleted {
		t.Error("applied an already-seen event; status should still be pending")
	}
	if s.LatestSeq != 10 {
		t.Errorf("LatestSeq = %d, want 10 (unchanged)", s.LatestSeq)
	}
}

func TestApply_MalformedPayloadReturnsError(t *testing.T) {
	s := NewState("")
	err := s.Apply(mustEvent(t, 1, EventTypeRoundStarted, `not json`))
	if err == nil {
		t.Fatal("expected error, got nil")
	}
}

func TestApply_DecodeErrorDoesNotAdvanceSeq(t *testing.T) {
	s := NewState("")
	// Malformed payload — Apply must return err AND leave LatestSeq at 0
	// so a retry (or snapshot-driven replay) can reach the same event.
	_ = s.Apply(mustEvent(t, 5, EventTypeRoundStarted, `not json`))
	if s.LatestSeq != 0 {
		t.Errorf("LatestSeq = %d, want 0 (decode failed — seq not consumed)", s.LatestSeq)
	}
	// A subsequent well-formed event with the same seq should apply.
	if err := s.Apply(mustEvent(t, 5, EventTypeRoundStarted, `{"round":1,"stage":"proposal"}`)); err != nil {
		t.Fatal(err)
	}
	if s.LatestSeq != 5 {
		t.Errorf("LatestSeq = %d, want 5", s.LatestSeq)
	}
}

func TestApply_ClassifierMetricsStoredRaw(t *testing.T) {
	s := NewState("")
	payload := `{"metrics":[{"id":"dissent_axis","name":"Dissent","scale":10,"mandatory":true}]}`
	_ = s.Apply(mustEvent(t, 1, EventTypeClassifierMetrics, payload))
	if string(s.ClassifierMetrics) != payload {
		t.Errorf("ClassifierMetrics = %s, want %s", s.ClassifierMetrics, payload)
	}
}
