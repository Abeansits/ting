package app

import (
	"fmt"
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"

	"github.com/Abeansits/ting/tui/internal/model"
)

// dissentAxisID is the metric id the classifier contract marks mandatory.
// The HTML dashboard renders it prominently; the TUI matches with reverse-
// video so the "always shown" invariant is visible at a glance.
const dissentAxisID = "dissent_axis"

const emDash = "—"

var spinnerFrames = []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}

// bar8 gives eighth-cell fractional fills so progress bars move smoothly.
var bar8 = []string{" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"}

var spark = []rune("▁▂▃▄▅▆▇█")

var (
	styleTitle       = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("212"))
	styleDim         = lipgloss.NewStyle().Faint(true)
	styleLabel       = lipgloss.NewStyle().Foreground(lipgloss.Color("244"))
	styleValue       = lipgloss.NewStyle().Foreground(lipgloss.Color("252"))
	styleHeading     = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("117"))
	styleBarFill     = lipgloss.NewStyle().Foreground(lipgloss.Color("114"))
	styleBarTrack    = lipgloss.NewStyle().Foreground(lipgloss.Color("238"))
	styleSpinner     = lipgloss.NewStyle().Foreground(lipgloss.Color("117"))
	stylePending     = lipgloss.NewStyle().Foreground(lipgloss.Color("240"))
	styleError       = lipgloss.NewStyle().Foreground(lipgloss.Color("203"))
	styleAccent      = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("215"))
	styleStatusDone  = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("114"))
	styleDissent     = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("219")).Reverse(true)
	styleHelpOverlay = lipgloss.NewStyle().
				Border(lipgloss.RoundedBorder()).
				BorderForeground(lipgloss.Color("244")).
				Padding(1, 2)
)

// Column widths for the rounds table. Header and body render from the same
// spec so padding changes in one place.
const (
	colStage     = 10
	colResponded = 10
)

func (m *Model) View() string {
	if m.showHelp {
		return m.renderHelp()
	}
	m.cache.refresh(m.state)

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
	line1 := fmt.Sprintf("%s  %s  %s  %s",
		styleTitle.Render("TING"),
		styleLabel.Render("·"),
		styleValue.Render(dashIfEmpty(s.ForumID)),
		statusBadge(s.Status),
	)
	meta := []string{
		fmt.Sprintf("Round %d/%d", m.cache.currentRoundNumber(), s.MaxRounds),
		fmt.Sprintf("elapsed %s", formatElapsed(s, m.now)),
		fmt.Sprintf("seq %d", s.LatestSeq),
	}
	return line1 + "\n" + styleDim.Render(strings.Join(meta, "  ·  "))
}

func (m *Model) renderTopic() string {
	s := m.state
	topic := styleHeading.Render("▸ " + dashIfEmpty(s.Topic))
	if len(s.Participants) == 0 {
		return topic
	}
	chips := make([]string, 0, len(s.Participants))
	for _, name := range s.Participants {
		chips = append(chips, m.participantChip(name))
	}
	return topic + "\n  " + strings.Join(chips, "   ")
}

func (m *Model) participantChip(name string) string {
	status := m.participantStatus(name)
	var glyph, label string
	switch status {
	case pStatusDone:
		glyph = styleBarFill.Render("✓")
		label = styleValue.Render(name)
	case pStatusActive:
		frame := spinnerFrames[m.spinnerIndex()]
		glyph = styleSpinner.Render(frame)
		label = styleValue.Render(name)
	default:
		glyph = stylePending.Render("·")
		label = styleLabel.Render(name)
	}
	return glyph + " " + label
}

type participantStatus int

const (
	pStatusPending participantStatus = iota
	pStatusActive
	pStatusDone
)

func (m *Model) participantStatus(name string) participantStatus {
	round := m.cache.activeRound()
	if round == nil {
		return pStatusPending
	}
	for _, r := range round.ParticipantsResponded {
		if r == name {
			return pStatusDone
		}
	}
	if m.state.Status == model.StatusInProgress {
		return pStatusActive
	}
	return pStatusPending
}

func (m *Model) renderRounds() string {
	var b strings.Builder
	b.WriteString(styleHeading.Render("ROUNDS"))
	b.WriteString("\n")

	rounds := m.cache.sortedRounds
	if len(rounds) == 0 {
		b.WriteString(styleDim.Render("  (rounds appear as the forum runs)"))
		return b.String()
	}

	header := fmt.Sprintf("   #  %s%s%s",
		padRight("STAGE", colStage),
		padRight("RESPONDED", colResponded),
		"CONVERGENCE",
	)
	b.WriteString(styleLabel.Render(header))
	b.WriteString("\n")

	focus := m.resolvedFocus()
	total := len(m.state.Participants)
	for _, r := range rounds {
		marker := "  "
		if int(r.Round) == focus {
			marker = styleAccent.Render("▸ ")
		}
		responded := fmt.Sprintf("%d/%d", len(r.ParticipantsResponded), total)
		conv := emDash
		if r.ConvergenceScore != nil {
			conv = fmt.Sprintf("%4.1f / 10", *r.ConvergenceScore)
		}
		fmt.Fprintf(&b, "%s%s  %s%s%s\n",
			marker,
			styleValue.Render(fmt.Sprintf("%d", r.Round)),
			styleValue.Render(padRight(r.Stage, colStage)),
			styleValue.Render(padRight(responded, colResponded)),
			styleValue.Render(conv),
		)
	}

	if active := m.cache.activeRound(); active != nil && total > 0 {
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

	if len(m.cache.classifier) == 0 {
		b.WriteString(styleDim.Render("  (axes appear after the pre-round classifier runs)"))
		return b.String()
	}

	nameWidth := 0
	for _, mt := range m.cache.classifier {
		if len(mt.Name) > nameWidth {
			nameWidth = len(mt.Name)
		}
	}

	for _, mt := range m.cache.classifier {
		history := m.cache.metricHistory[mt.ID]
		scale := mt.Scale
		if scale <= 0 {
			scale = 10
		}

		name := padRight(mt.Name, nameWidth)
		if mt.ID == dissentAxisID || mt.Mandatory {
			name = styleDissent.Render(name)
		} else {
			name = styleValue.Render(name)
		}

		var bar, latest string
		if len(history) == 0 {
			bar = styleBarTrack.Render(strings.Repeat("░", 12))
			latest = styleDim.Render(emDash)
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

	latest := m.state.ConvergenceScore
	if latest == nil && len(m.cache.convergence) > 0 {
		v := m.cache.convergence[len(m.cache.convergence)-1]
		latest = &v
	}
	if latest == nil {
		b.WriteString(styleDim.Render("  (awaiting first judge score)"))
		return b.String()
	}

	b.WriteString("  ")
	b.WriteString(styleLabel.Render("latest "))
	b.WriteString(renderBar(*latest, 10, 24))
	b.WriteString("  ")
	b.WriteString(styleValue.Render(fmt.Sprintf("%4.1f / 10", *latest)))
	b.WriteString("\n")
	if len(m.cache.convergence) > 1 {
		parts := make([]string, 0, len(m.cache.convergence))
		for _, h := range m.cache.convergence {
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

	if len(m.cache.synthesis) == 0 {
		b.WriteString(styleDim.Render("  (no synthesis yet)"))
		return b.String()
	}
	focus := m.resolvedFocus()
	for _, s := range m.cache.synthesis {
		marker := "  "
		if int(s.Round) == focus {
			marker = styleAccent.Render("▸ ")
		}
		fmt.Fprintf(&b, "%sround %d  %s\n",
			marker,
			s.Round,
			styleDim.Render(fmt.Sprintf("%d words", s.Words)),
		)
	}
	return b.String()
}

func (m *Model) renderFooter() string {
	hints := []string{
		"q quit",
		"r refresh",
		"? help",
		"↑/↓ focus round",
		"1-9 jump",
		"0 latest",
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
		"  ↓ / j              focus next round (wraps to latest)",
		"  1 … 9              jump focus to a specific round",
		"  0                  clear focus (follow latest)",
		"",
		styleDim.Render("press ? to close"),
	}, "\n")
	return styleHelpOverlay.Render(body)
}

// resolvedFocus returns the round the user is focused on, defaulting to the
// latest round when no explicit focus is set.
func (m *Model) resolvedFocus() int {
	if m.focusedRound > 0 {
		return m.focusedRound
	}
	return int(m.cache.currentRoundNumber())
}

// spinnerIndex samples a spinner frame from m.now so the animation advances
// even if a tick is dropped; falling back to 0 when now is zero value (tests).
func (m *Model) spinnerIndex() int {
	if m.now.IsZero() {
		return 0
	}
	return int(m.now.UnixMilli()/int64(tickInterval/time.Millisecond)) % len(spinnerFrames)
}

func statusBadge(st model.Status) string {
	switch st {
	case model.StatusCompleted:
		return styleStatusDone.Render("completed")
	case model.StatusInProgress:
		return styleAccent.Render("in_progress")
	default:
		return stylePending.Render("pending")
	}
}

func formatElapsed(s *model.State, now time.Time) string {
	if s.Created.IsZero() {
		return emDash
	}
	end := s.Updated
	if s.Status == model.StatusInProgress || end.IsZero() {
		end = now
	}
	d := end.Sub(s.Created)
	if d < 0 {
		d = 0
	}
	return fmt.Sprintf("%dm %02ds", int(d.Minutes()), int(d.Seconds())%60)
}

// renderBar draws a horizontal progress bar using eighth-block glyphs for
// smooth fractional fills.
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

// renderSparkline draws a per-round score trace. Missing rounds render as
// space so the trace stays anchored to the full round count.
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
		ratio := values[i] / scale
		switch {
		case scale <= 0, ratio < 0:
			ratio = 0
		case ratio > 1:
			ratio = 1
		}
		b.WriteRune(spark[int(ratio*float64(len(spark)-1))])
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
		return emDash
	}
	return s
}
