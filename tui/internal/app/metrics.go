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

type synthEntry struct {
	Round uint32
	Words int
}

// viewCache holds once-per-state-change derivations so the 150ms render tick
// doesn't re-sort rounds or re-unmarshal payloads every frame. Invalidated by
// comparing seq to state.LatestSeq.
type viewCache struct {
	seq            uint64
	sortedRounds   []model.RoundSummary
	classifier     []metricDef
	metricHistory  map[string][]float64
	convergence    []float64
	synthesis      []synthEntry
	latestRoundIdx int
}

// refresh rebuilds the cache if the state has advanced since the last build.
// A zero-valued cache (seq=0) is treated as stale so first-paint is always
// a full rebuild — otherwise an empty initial State would appear cached.
func (c *viewCache) refresh(s *model.State) {
	if c.seq != 0 && c.seq == s.LatestSeq && len(c.sortedRounds) == len(s.Rounds) {
		return
	}
	c.seq = s.LatestSeq
	c.sortedRounds = append(c.sortedRounds[:0], s.Rounds...)
	sort.Slice(c.sortedRounds, func(i, j int) bool {
		return c.sortedRounds[i].Round < c.sortedRounds[j].Round
	})
	c.latestRoundIdx = len(c.sortedRounds) - 1

	c.classifier = decodeClassifier(s.ClassifierMetrics)

	if c.metricHistory == nil {
		c.metricHistory = make(map[string][]float64, len(c.classifier))
	}
	for k := range c.metricHistory {
		delete(c.metricHistory, k)
	}
	for _, md := range c.classifier {
		c.metricHistory[md.ID] = metricHistory(c.sortedRounds, md.ID)
	}

	c.convergence = convergenceHistory(c.sortedRounds)
	c.synthesis = synthesisEntries(c.sortedRounds)
}

func (c *viewCache) activeRound() *model.RoundSummary {
	if c.latestRoundIdx < 0 {
		return nil
	}
	return &c.sortedRounds[c.latestRoundIdx]
}

func (c *viewCache) currentRoundNumber() uint32 {
	if c.latestRoundIdx < 0 {
		return 0
	}
	return c.sortedRounds[c.latestRoundIdx].Round
}

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
	history := make([]float64, 0, len(rounds))
	for _, r := range rounds {
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

func convergenceHistory(rounds []model.RoundSummary) []float64 {
	out := make([]float64, 0, len(rounds))
	for _, r := range rounds {
		if r.ConvergenceScore != nil {
			out = append(out, *r.ConvergenceScore)
		}
	}
	return out
}

func synthesisEntries(rounds []model.RoundSummary) []synthEntry {
	out := make([]synthEntry, 0, len(rounds))
	for _, r := range rounds {
		if len(r.Synthesis) == 0 {
			continue
		}
		var p struct {
			Round     uint32 `json:"round"`
			WordCount int    `json:"word_count"`
		}
		if err := json.Unmarshal(r.Synthesis, &p); err != nil {
			continue
		}
		out = append(out, synthEntry{Round: p.Round, Words: p.WordCount})
	}
	return out
}
