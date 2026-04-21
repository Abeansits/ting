package model

import (
	"encoding/json"
	"fmt"
	"time"
)

const StateVersion uint32 = 1

type Status string

const (
	StatusPending    Status = "pending"
	StatusInProgress Status = "in_progress"
	StatusCompleted  Status = "completed"
)

// RoundSummary mirrors the snapshot's per-round shape. Loosely-typed payload
// fields are stored as RawMessage so the view layer can decode what it needs
// without forcing the reducer to mirror every payload field.
type RoundSummary struct {
	Round                 uint32          `json:"round"`
	Stage                 string          `json:"stage"`
	ParticipantsResponded []string        `json:"participants_responded"`
	Synthesis             json.RawMessage `json:"synthesis,omitempty"`
	MetricScores          json.RawMessage `json:"metric_scores,omitempty"`
	ConvergenceScore      *float64        `json:"convergence_score,omitempty"`
}

// State mirrors dashboard-state.json.
type State struct {
	Version           uint32          `json:"version"`
	ForumID           string          `json:"forum_id"`
	Topic             string          `json:"topic"`
	Participants      []string        `json:"participants"`
	MaxRounds         uint32          `json:"max_rounds"`
	Created           time.Time       `json:"created"`
	Updated           time.Time       `json:"updated"`
	LatestSeq         uint64          `json:"latest_seq"`
	Status            Status          `json:"status"`
	Rounds            []RoundSummary  `json:"rounds"`
	ClassifierMetrics json.RawMessage `json:"classifier_metrics,omitempty"`
	ConvergenceScore  *float64        `json:"convergence_score,omitempty"`
}

// NewState returns an empty state for the case where no snapshot exists yet.
func NewState(forumID string) *State {
	return &State{
		Version:      StateVersion,
		ForumID:      forumID,
		Participants: []string{},
		Status:       StatusPending,
		Rounds:       []RoundSummary{},
	}
}

// Apply folds one event into the state. Events with seq <= LatestSeq are
// skipped: the snapshot the tailer starts from may already include them, and
// idempotency lets us replay without double-counting. Unknown event types are
// a no-op per schemas/CONTRACT.md.
func (s *State) Apply(e Event) error {
	if e.Seq <= s.LatestSeq {
		return nil
	}

	switch e.Type {
	case EventTypeForumStarted:
		var p struct {
			Topic        string   `json:"topic"`
			Participants []string `json:"participants"`
			MaxRounds    uint32   `json:"max_rounds"`
		}
		if err := json.Unmarshal(e.Payload, &p); err != nil {
			return payloadErr(e.Type, err)
		}
		s.ForumID = e.ForumID
		s.Topic = p.Topic
		s.Participants = p.Participants
		s.MaxRounds = p.MaxRounds
		if s.Status == StatusPending {
			s.Status = StatusInProgress
		}
	case EventTypeRoundStarted:
		var p struct {
			Round uint32 `json:"round"`
			Stage string `json:"stage"`
		}
		if err := json.Unmarshal(e.Payload, &p); err != nil {
			return payloadErr(e.Type, err)
		}
		s.ensureRound(p.Round).Stage = p.Stage
	case EventTypeParticipantResponse:
		var p struct {
			Round       uint32 `json:"round"`
			Participant string `json:"participant"`
		}
		if err := json.Unmarshal(e.Payload, &p); err != nil {
			return payloadErr(e.Type, err)
		}
		r := s.ensureRound(p.Round)
		r.ParticipantsResponded = append(r.ParticipantsResponded, p.Participant)
	case EventTypeSynthesis:
		var p struct {
			Round uint32 `json:"round"`
		}
		if err := json.Unmarshal(e.Payload, &p); err != nil {
			return payloadErr(e.Type, err)
		}
		s.ensureRound(p.Round).Synthesis = e.Payload
	case EventTypeMetricScores:
		var p struct {
			Round uint32 `json:"round"`
		}
		if err := json.Unmarshal(e.Payload, &p); err != nil {
			return payloadErr(e.Type, err)
		}
		s.ensureRound(p.Round).MetricScores = e.Payload
	case EventTypeConvergence:
		var p struct {
			Round uint32  `json:"round"`
			Score float64 `json:"score"`
		}
		if err := json.Unmarshal(e.Payload, &p); err != nil {
			return payloadErr(e.Type, err)
		}
		score := p.Score
		s.ensureRound(p.Round).ConvergenceScore = &score
		s.ConvergenceScore = &score
	case EventTypeClassifierMetrics:
		s.ClassifierMetrics = e.Payload
	case EventTypeForumComplete:
		s.Status = StatusCompleted
	case EventTypeClaims, EventTypeAlignment:
		// Not carried in the snapshot shape.
	}

	// Bump seq only after a successful decode so a malformed payload can
	// be retried against the same state rather than silently consuming it.
	s.LatestSeq = e.Seq
	s.Updated = e.Timestamp
	return nil
}

func (s *State) ensureRound(round uint32) *RoundSummary {
	for i := range s.Rounds {
		if s.Rounds[i].Round == round {
			return &s.Rounds[i]
		}
	}
	s.Rounds = append(s.Rounds, RoundSummary{
		Round:                 round,
		ParticipantsResponded: []string{},
	})
	return &s.Rounds[len(s.Rounds)-1]
}

func payloadErr(t EventType, err error) error {
	return fmt.Errorf("decode %s payload: %w", t, err)
}
