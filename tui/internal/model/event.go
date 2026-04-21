// Package model mirrors the cross-language wire contract defined in
// schemas/dashboard-event.schema.json and schemas/dashboard-state.schema.json.
// See schemas/CONTRACT.md for reader/writer rules.
package model

import (
	"encoding/json"
	"errors"
	"fmt"
	"time"
)

const EventVersion uint32 = 1

type EventType string

const (
	EventTypeForumStarted        EventType = "forum_started"
	EventTypeRoundStarted        EventType = "round_started"
	EventTypeParticipantResponse EventType = "participant_response"
	EventTypeSynthesis           EventType = "synthesis"
	EventTypeClaims              EventType = "claims"
	EventTypeAlignment           EventType = "alignment"
	EventTypeClassifierMetrics   EventType = "classifier_metrics"
	EventTypeMetricScores        EventType = "metric_scores"
	EventTypeConvergence         EventType = "convergence"
	EventTypeForumComplete       EventType = "forum_complete"
)

// Event is one line of dashboard-events.jsonl. Payload is kept as RawMessage
// so unknown payload fields round-trip unchanged and per-type decoders can
// live where they're consumed (reducer in state.go).
type Event struct {
	Version   uint32          `json:"version"`
	Seq       uint64          `json:"seq"`
	ForumID   string          `json:"forum_id"`
	Timestamp time.Time       `json:"timestamp"`
	Type      EventType       `json:"type"`
	Payload   json.RawMessage `json:"payload"`
}

// ErrUnknownEventType is returned alongside a filled Event when the envelope
// parses but carries a Type the reader doesn't recognise. Per CONTRACT.md
// readers MUST skip-and-warn rather than fail.
var ErrUnknownEventType = errors.New("unknown event type")

// ParseEvent decodes one JSONL line into an Event. Malformed JSON or missing
// envelope fields return an error and the zero Event. Unknown Type returns
// the parsed Event plus ErrUnknownEventType so the caller can log the raw
// type string before skipping.
func ParseEvent(line []byte) (Event, error) {
	var e Event
	if err := json.Unmarshal(line, &e); err != nil {
		return Event{}, fmt.Errorf("parse event: %w", err)
	}
	if e.Version == 0 || e.Seq == 0 || e.ForumID == "" || e.Type == "" || len(e.Payload) == 0 {
		return Event{}, errors.New("missing required envelope fields")
	}
	if !IsKnownEventType(e.Type) {
		return e, ErrUnknownEventType
	}
	return e, nil
}

func IsKnownEventType(t EventType) bool {
	switch t {
	case EventTypeForumStarted, EventTypeRoundStarted, EventTypeParticipantResponse,
		EventTypeSynthesis, EventTypeClaims, EventTypeAlignment,
		EventTypeClassifierMetrics, EventTypeMetricScores, EventTypeConvergence,
		EventTypeForumComplete:
		return true
	}
	return false
}
