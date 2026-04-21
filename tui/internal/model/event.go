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

// Event is one line of dashboard-events.jsonl. Payload stays as RawMessage
// so unknown fields round-trip and per-type decoders live in the reducer.
type Event struct {
	Version   uint32          `json:"version"`
	Seq       uint64          `json:"seq"`
	ForumID   string          `json:"forum_id"`
	Timestamp time.Time       `json:"timestamp"`
	Type      EventType       `json:"type"`
	Payload   json.RawMessage `json:"payload"`
}

// ErrUnknownEventType is returned with a filled Event when the envelope parses
// but the Type is unrecognised. Per schemas/CONTRACT.md readers skip-and-warn.
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
	switch {
	case e.Version == 0:
		return Event{}, errors.New("envelope missing version")
	case e.Seq == 0:
		return Event{}, errors.New("envelope missing seq")
	case e.ForumID == "":
		return Event{}, errors.New("envelope missing forum_id")
	case e.Type == "":
		return Event{}, errors.New("envelope missing type")
	case len(e.Payload) == 0:
		return Event{}, errors.New("envelope missing payload")
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
