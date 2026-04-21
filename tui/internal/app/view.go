package app

import (
	"encoding/json"
	"fmt"
	"sort"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"

	"github.com/Abeansits/ting/tui/internal/model"
)

// Dissent Axis is the classifier metric the plan marks mandatory. We render
// it with inverted color to honor that status in the TUI the same way the
// HTML dashboard does.
const dissentAxisID = "dissent_axis"

var spinnerFrames = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

// Box-drawing blocks for bars. Eighths give smooth fractional widths.
var bar8 = []string{" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"}

// Sparkline glyphs at eight levels.
var spark = []rune("▁▂▃▄▅▆▇█")

var (
	styleTitle       = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("212"))
	styleDim         = lipgloss.NewStyle().Faint(true)
	styleLabel       = lipgloss.NewStyle().Foreground(lipgloss.Color("244"))
	styleValue       = lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	styleHeading     = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("117"))
	styleBarFill     = lipgloss.NewStyle().Foreground(lipgloss.Color("114"))
	styleBarTrack    = lipgloss.NewStyle().Foreground(lipgloss.Color("238"))
	styleOK          = lipgloss.NewStyle().Foreground(lipgloss.Color("114"))
	styleSpinner     = lipgloss.NewStyle().Foreground(lipgloss.Color("117"))
	stylePending     = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	styleError       = lipgloss.NewStyle().Foreground(lipgloss.Color("203"))
	styleStatus      = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("215"))
	styleStatusDone  = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("114"))
	styleDissent     = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("219")).Reverse(true)
	styleFocusMark   = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("215"))
	styleHelpOverlay = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(lipgloss.Color("244")).
				Padding(1, 2)
)

func (m *Model) View() string {
	if m.showHelp {
		return m.renderHelp()
	}

	var b strings.Builder
	b.WriteString(m.renderHeader())
	b.WriteString("\n\n")
	b.WriteString(m.renderTopic())
	b.WriteString("\n\n")
	b.WriteString(m.renderRounds())
	b.WriteString("\n")
	b.WriteString(m.renderMetrics())
	b.WriteString("\n")
	b.WriteString(m.renderConvergence())
	b.WriteString("\n")
	b.WriteString(m.renderSynthesis())

	if m.loadErr != nil {
		b.WriteString("\n")
		b.WriteString(styleError.Render(fmt.Sprintf("snapshot load error: %v", m.loadErr)))
	}
	if m.tailErr != nil {
		b.WriteString("\n")
		b.WriteString(styleError.Render(fmt.Sprintf("tailer error: %v", m.tailErr)))
	}

	b.WriteString("\n\n")
	b.WriteString(m.renderFooter())
	return b.String()
}

func (m *Model) renderHeader() string {
	s := m.state
	forum := dashIfEmpty(s.ForumID)
	status := statusBadge(s.Status)
	round := fmt.Sprintf("Round %d/%d", currentRound(s), s.MaxRounds)
	elapsed := fmt.Sprintf("elapsed %s", formatElapsed(s, m.now))
	seq := fmt.Sprintf("seq %d", s.LatestSeq)

	line1 := fmt.Sprintf("%s  %s  %s  %s",
		styleTitle.Render("TING"),
		styleLabel.Render("·"),
		styleValue.Render(forum),
		status,
	)
	line2 := styleDim.Render(strings.Join([]string{round, elapsed, seq}, "  ·  "))
	return line1 + "\n" + line2
}

func (m *Model) renderTopic() string {
	s := m.state
	topic := styleHeading.Render("▸ " + dashIfEmpty(s.Topic))

	if len(s.Participants) == 0 {
		return topic
	}
	parts := make([]string, 0, len(s.Participants))
	for _, name := range s.Participants {
		parts = append(parts, m.participantChip(name))
	}
	return topic + "\n  " + strings.Join(parts, "   ")
}

// participantChip renders one participant with their current-round status
// glyph. A spinner frame is chosen from m.tick so active participants animate.
func (m *Model) participantChip(name string) string {
	status, active := m.participantStatus(name)
	var glyph string
	switch status {
	case pStatusDone:
		glyph = styleOK.Render("✓")
	case pStatusActive:
		glyph = styleSpinner.Render(spinnerFrames[int(m.tick)%len(spinnerFrames)])
	default:
		glyph = stylePending.Render("·")
	}
	if active {
		return fmt.Sprintf("%s %s", glyph, styleValue.Render(name))
	}
	return fmt.Sprintf("%s %s", glyph, styleLabel.Render(name))
}

type participantStatus int

const (
	pStatusPending participantStatus = iota
	pStatusActive
	pStatusDone
)

// participantStatus reports how one participant stands in the current round:
// done (responded), active (round in progress and not yet responded), or
// pending (no active round).
func (m *Model) participantStatus(name string) (participantStatus, bool) {
	round := activeRound(m.state)
	if round == nil {
		return pStatusPending, false
	}
	for _, r := range round.ParticipantsResponded {
		if r == name {
			return pStatusDone, true
		}
	}
	if m.state.Status == model.StatusInProgress {
		return pStatusActive, true
	}
	return pStatusPending, false
}

func (m *Model) renderRounds() string {
	var b strings.Builder
	b.WriteString(styleHeading.Render("ROUNDS"))
	b.WriteString("\n")
	if len(m.state.Rounds) == 0 {
		b.WriteString(styleDim.Render("  (rounds appear as the forum runs)"))
		return b.String()
	}
	b.WriteString(styleLabel.Render("   #  STAGE      RESPONDED  CONVERGENCE"))
	b.WriteString("\n")

	sorted := make([]model.RoundSummary, len(m.state.Rounds))
	copy(sorted, m.state.Rounds)
	sort.Slice(sorted, func(i, j int) bool { return sorted[i].Round < sorted[j].Round })

	focus := m.resolvedFocus()
	total := len(m.state.Participants)
	for _, r := range sorted {
		marker := "  "
		if int(r.Round) == focus {
			marker = styleFocusMark.Render("▸ ")
		}
		stage := padRight(r.Stage, 9)
		responded := fmt.Sprintf("%d/%d", len(r.ParticipantsResponded), total)
		conv := "—"
		if r.ConvergenceScore != nil {
			conv = fmt.Sprintf("%4.1f / 10", *r.ConvergenceScore)
		}
		fmt.Fprintf(&b, "%s%s  %s  %s  %s\n",
			marker,
			styleValue.Render(fmt.Sprintf("%d", r.Round)),
			styleValue.Render(stage),
			styleValue.Render(padRight(responded, 9)),
			styleValue.Render(conv),
		)
	}

	if active := activeRound(m.state); active != nil && total > 0 {
		b.WriteString("  ")
		b.WriteString(styleLabel.Render("round progress "))
		b.WriteString(renderBar(float64(len(active.ParticipantsResponded)), float64(total), 20))
		b.WriteString(styleDim.Render(fmt.Sprintf(" %d/%d", len(active.ParticipantsResponded), total)))
		b.WriteString("\n")
	}

	return b.String()
}

func (m *Model) renderMetrics() string {
	var b strings.Builder
	b.WriteString(styleHeading.Render("METRICS"))
	b.WriteString("  ")
	b.WriteString(styleDim.Render("(Dissent Axis always shown)"))
	b.WriteString("\n")

	metrics := decodeClassifier(m.state.ClassifierMetrics)
	if len(metrics) == 0 {
		b.WriteString(styleDim.Render("  (axes appear after the pre-round classifier runs)"))
		return b.String()
	}

	nameWidth := 0
	for _, mt := range metrics {
		if len(mt.Name) > nameWidth {
			nameWidth = len(mt.Name)
		}
	}

	for _, mt := range metrics {
		history := metricHistory(m.state.Rounds, mt.ID)
		scale := mt.Scale
		if scale <= 0 {
			scale = 10
		}

		name := padRight(mt.Name, nameWidth)
		if mt.ID == dissentAxisID || mt.Mandatory {
			name = styleDissent.Render(padRight(mt.Name, nameWidth))
		} else {
			name = styleValue.Render(name)
		}

		var latest string
		var bar string
		if len(history) == 0 {
			bar = styleBarTrack.Render(strings.Repeat("░", 12))
			latest = styleDim.Render("—")
		} else {
			last := history[len(history)-1]
			bar = renderBar(last, float64(scale), 12)
			latest = styleValue.Render(fmt.Sprintf("%4.1f / %d", last, scale))
		}

		sparkline := renderSparkline(history, float64(scale), m.state.MaxRounds)

		fmt.Fprintf(&b, "  %s  %s  %s  %s\n", name, bar, sparkline, latest)
	}
	return b.String()
}

func (m *Model) renderConvergence() string {
	var b strings.Builder
	b.WriteString(styleHeading.Render("CONVERGENCE"))
	b.WriteString("\n")
	history := convergenceHistory(m.state.Rounds)
	latest := m.state.ConvergenceScore
	if latest == nil && len(history) > 0 {
		v := history[len(history)-1]
		latest = &v
	}
	if latest == nil {
		b.WriteString(styleDim.Render("  (awaiting first judge score)"))
		return b.String()
	}
	bar := renderBar(*latest, 10, 24)
	b.WriteString("  ")
	b.WriteString(styleLabel.Render("latest "))
	b.WriteString(bar)
	b.WriteString("  ")
	b.WriteString(styleValue.Render(fmt.Sprintf("%4.1f / 10", *latest)))
	b.WriteString("\n")
	if len(history) > 1 {
		parts := make([]string, 0, len(history))
		for _, h := range history {
			parts = append(parts, fmt.Sprintf("%.1f", h))
		}
		b.WriteString("  ")
		b.WriteString(styleLabel.Render("history "))
		b.WriteString(styleValue.Render(strings.Join(parts, " → ")))
		b.WriteString("\n")
	}
	return b.String()
}

func (m *Model) renderSynthesis() string {
	var b strings.Builder
	b.WriteString(styleHeading.Render("SYNTHESIS"))
	b.WriteString("\n")

	type synthEntry struct {
		round uint32
		words int
	}
	synths := make([]synthEntry, 0, len(m.state.Rounds))
	for _, r := range m.state.Rounds {
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
		synths = append(synths, synthEntry{round: p.Round, words: p.WordCount})
	}
	if len(synths) == 0 {
		b.WriteString(styleDim.Render("  (no synthesis yet)"))
		return b.String()
	}
	sort.Slice(synths, func(i, j int) bool { return synths[i].round < synths[j].round })

	focus := m.resolvedFocus()
	for _, s := range synths {
		marker := "  "
		if int(s.round) == focus {
			marker = styleFocusMark.Render("▸ ")
		}
		fmt.Fprintf(&b, "%sround %d  %s\n",
			marker,
			s.round,
			styleDim.Render(fmt.Sprintf("%d words", s.words)),
		)
	}
	return b.String()
}

func (m *Model) renderFooter() string {
	hints := []string{
		"q quit",
		"r refresh",
		"?" + " help",
		"↑/↓ focus round",
		"1-9 jump",
	}
	return styleDim.Render(strings.Join(hints, "  ·  "))
}

func (m *Model) renderHelp() string {
	body := strings.Join([]string{
		styleTitle.Render("Ting TUI — keyboard"),
		"",
		"  q / Ctrl-C / Esc   quit",
		"  r                  reload dashboard-state.json from disk",
		"  ?                  toggle this help",
		"  ↑ / k              focus previous round",
		"  ↓ / j              focus next round (0 = latest)",
		"  1 … 9              jump focus to a specific round",
		"",
		styleDim.Render("press ? to close"),
	}, "\n")
	return styleHelpOverlay.Render(body)
}

// resolvedFocus returns the absolute round number the user is focused on,
// defaulting to the latest (active) round if 0.
func (m *Model) resolvedFocus() int {
	if m.focusedRound > 0 {
		return m.focusedRound
	}
	if r := activeRound(m.state); r != nil {
		return int(r.Round)
	}
	return 0
}

// currentRound returns the highest round number present, or 0.
func currentRound(s *model.State) uint32 {
	var max uint32
	for _, r := range s.Rounds {
		if r.Round > max {
			max = r.Round
		}
	}
	return max
}

// activeRound returns the round still in progress (or latest if forum
// completed). nil when no rounds yet.
func activeRound(s *model.State) *model.RoundSummary {
	if len(s.Rounds) == 0 {
		return nil
	}
	idx := 0
	for i := range s.Rounds {
		if s.Rounds[i].Round > s.Rounds[idx].Round {
			idx = i
		}
	}
	return &s.Rounds[idx]
}

func statusBadge(st model.Status) string {
	switch st {
	case model.StatusCompleted:
		return styleStatusDone.Render("completed")
	case model.StatusInProgress:
		return styleStatus.Render("in_progress")
	default:
		return stylePending.Render("pending")
	}
}

func formatElapsed(s *model.State, now time.Time) string {
	if s.Created.IsZero() {
		return "—"
	}
	end := s.Updated
	if s.Status == model.StatusInProgress || end.IsZero() {
		end = now
	}
	d := end.Sub(s.Created)
	if d < 0 {
		d = 0
	}
	mins := int(d.Minutes())
	secs := int(d.Seconds()) % 60
	return fmt.Sprintf("%dm %02ds", mins, secs)
}

// renderBar draws a horizontal progress bar of the given cell width, using
// eighth-block glyphs for smooth fractional fills.
func renderBar(value, scale float64, width int) string {
	if width <= 0 || scale <= 0 {
		return ""
	}
	ratio := value / scale
	switch {
	case ratio < 0:
		ratio = 0
	case ratio > 1:
		ratio = 1
	}
	eighths := int(ratio * float64(width) * 8)
	full := eighths / 8
	rem := eighths % 8
	var b strings.Builder
	if full > 0 {
		b.WriteString(styleBarFill.Render(strings.Repeat("█", full)))
	}
	if rem > 0 && full < width {
		b.WriteString(styleBarFill.Render(bar8[rem]))
		full++
	}
	if full < width {
		b.WriteString(styleBarTrack.Render(strings.Repeat("░", width-full)))
	}
	return b.String()
}

// renderSparkline draws a per-round score trace. Missing leading/trailing
// rounds show as space so the line stays anchored to total rounds.
func renderSparkline(values []float64, scale float64, totalRounds uint32) string {
	width := int(totalRounds)
	if width < len(values) {
		width = len(values)
	}
	if width == 0 {
		return ""
	}
	var b strings.Builder
	for i := 0; i < width; i++ {
		if i >= len(values) {
			b.WriteRune(' ')
			continue
		}
		v := values[i]
		if scale <= 0 {
			b.WriteRune(spark[0])
			continue
		}
		ratio := v / scale
		switch {
		case ratio < 0:
			ratio = 0
		case ratio > 1:
			ratio = 1
		}
		idx := int(ratio * float64(len(spark)-1))
		b.WriteRune(spark[idx])
	}
	return styleBarFill.Render(b.String())
}

func padRight(s string, n int) string {
	if len(s) >= n {
		return s
	}
	return s + strings.Repeat(" ", n-len(s))
}

func dashIfEmpty(s string) string {
	if s == "" {
		return "—"
	}
	return s
}
