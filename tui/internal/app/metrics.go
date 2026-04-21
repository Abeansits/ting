package app

import (
	"encoding/json"
	"sort"

	"github.com/Abeansits/ting/tui/internal/model"
)

// metricDef is the classifier's declaration of one axis.
type metricDef struct {
	ID        string `json:"id"`
	Name      string `json:"name"`
	Scale     int    `json:"scale"`
	Mandatory bool   `json:"mandatory,omitempty"`
}

type classifierPayload struct {
	Metrics []metricDef `json:"metrics"`
}

type scoredMetric struct {
	MetricID string  `json:"metric_id"`
	Score    float64 `json:"score"`
}

type metricScoresPayload struct {
	Round  uint32         `json:"round"`
	Scores []scoredMetric `json:"scores"`
}

// decodeClassifier returns the metric axes declared for the forum. Empty slice
// if not yet emitted or unparsable.
func decodeClassifier(raw json.RawMessage) []metricDef {
	if len(raw) == 0 {
		return nil
	}
	var p classifierPayload
	if err := json.Unmarshal(raw, &p); err != nil {
		return nil
	}
	return p.Metrics
}

// metricHistory collects per-round scores for one metric id, in round order.
// Missing rounds are skipped; callers align by position, not round number.
func metricHistory(rounds []model.RoundSummary, metricID string) []float64 {
	sorted := make([]model.RoundSummary, len(rounds))
	copy(sorted, rounds)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Round < sorted[j].Round })

	history := make([]float64, 0, len(sorted))
	for _, r := range sorted {
		if len(r.MetricScores) == 0 {
			continue
		}
		var p metricScoresPayload
		if err := json.Unmarshal(r.MetricScores, &p); err != nil {
			continue
		}
		for _, s := range p.Scores {
			if s.MetricID == metricID {
				history = append(history, s.Score)
				break
			}
		}
	}
	return history
}

// convergenceHistory returns per-round convergence scores in round order.
func convergenceHistory(rounds []model.RoundSummary) []float64 {
	sorted := make([]model.RoundSummary, len(rounds))
	copy(sorted, rounds)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Round < sorted[j].Round })

	out := make([]float64, 0, len(sorted))
	for _, r := range sorted {
		if r.ConvergenceScore != nil {
			out = append(out, *r.ConvergenceScore)
		}
	}
	return out
}
