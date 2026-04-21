package model

import (
	"errors"
	"testing"
)

func TestParseEvent_ValidAllTypes(t *testing.T) {
	cases := []struct {
		name string
		line string
		want EventType
	}{
		{"forum_started", `{"version":1,"seq":1,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"forum_started","payload":{"topic":"t","participants":["a"],"max_rounds":3}}`, EventTypeForumStarted},
		{"round_started", `{"version":1,"seq":2,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"round_started","payload":{"round":1,"stage":"proposal"}}`, EventTypeRoundStarted},
		{"participant_response", `{"version":1,"seq":3,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"participant_response","payload":{"round":1,"participant":"codex"}}`, EventTypeParticipantResponse},
		{"synthesis", `{"version":1,"seq":4,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"synthesis","payload":{"round":1,"word_count":220}}`, EventTypeSynthesis},
		{"claims", `{"version":1,"seq":5,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"claims","payload":{"round":1,"claims":[]}}`, EventTypeClaims},
		{"alignment", `{"version":1,"seq":6,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"alignment","payload":{"round":1}}`, EventTypeAlignment},
		{"classifier_metrics", `{"version":1,"seq":7,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"classifier_metrics","payload":{"metrics":[]}}`, EventTypeClassifierMetrics},
		{"metric_scores", `{"version":1,"seq":8,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"metric_scores","payload":{"round":1,"scores":[]}}`, EventTypeMetricScores},
		{"convergence", `{"version":1,"seq":9,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"convergence","payload":{"round":1,"score":6.1}}`, EventTypeConvergence},
		{"forum_complete", `{"version":1,"seq":10,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"forum_complete","payload":{"rounds_used":2}}`, EventTypeForumComplete},
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			got, err := ParseEvent([]byte(tc.line))
			if err != nil {
				t.Fatalf("ParseEvent: %v", err)
			}
			if got.Type != tc.want {
				t.Errorf("Type = %q, want %q", got.Type, tc.want)
			}
			if got.Version != 1 {
				t.Errorf("Version = %d, want 1", got.Version)
			}
			if got.Seq == 0 {
				t.Errorf("Seq = 0, want nonzero")
			}
		})
	}
}

func TestParseEvent_UnknownTypeReturnsSentinel(t *testing.T) {
	line := `{"version":1,"seq":1,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"brand_new_event","payload":{}}`
	got, err := ParseEvent([]byte(line))
	if !errors.Is(err, ErrUnknownEventType) {
		t.Fatalf("expected ErrUnknownEventType, got %v", err)
	}
	// The envelope should still be filled so the caller can log the raw type.
	if got.Type != "brand_new_event" {
		t.Errorf("Type = %q, want brand_new_event", got.Type)
	}
}

func TestParseEvent_MalformedJSON(t *testing.T) {
	if _, err := ParseEvent([]byte("not json at all")); err == nil {
		t.Fatal("expected error on malformed JSON, got nil")
	}
}

func TestParseEvent_MissingEnvelopeFields(t *testing.T) {
	cases := []string{
		`{}`,
		`{"version":1,"seq":1,"forum_id":"f"}`, // missing type/timestamp/payload
		`{"version":0,"seq":1,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"round_started","payload":{}}`,
		`{"version":1,"seq":0,"forum_id":"f","timestamp":"2026-04-19T13:55:00Z","type":"round_started","payload":{}}`,
		`{"version":1,"seq":1,"forum_id":"","timestamp":"2026-04-19T13:55:00Z","type":"round_started","payload":{}}`,
	}
	for i, line := range cases {
		if _, err := ParseEvent([]byte(line)); err == nil {
			t.Errorf("case %d: expected error, got nil for %q", i, line)
		}
	}
}

func TestIsKnownEventType(t *testing.T) {
	if !IsKnownEventType(EventTypeForumStarted) {
		t.Error("forum_started should be known")
	}
	if IsKnownEventType("bogus") {
		t.Error("bogus should not be known")
	}
}
