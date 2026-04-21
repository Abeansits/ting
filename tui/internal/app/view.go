package app

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// Phase 3A ships a deliberately thin view: enough signal that the tailer is
// wired up correctly, nothing more. 3B replaces the body with the real
// dashboard (header, round table, metrics bars, Dissent Axis, convergence
// gauge, synthesis preview).

var headerStyle = lipgloss.NewStyle().Bold(true).Foreground(lipgloss.Color("212"))

func (m *Model) View() string {
	var b strings.Builder
	fmt.Fprintln(&b, headerStyle.Render("Ting TUI — Phase 3A skeleton"))

	s := m.state
	fmt.Fprintf(&b, "forum:   %s\n", dashIfEmpty(s.ForumID))
	fmt.Fprintf(&b, "topic:   %s\n", dashIfEmpty(s.Topic))
	fmt.Fprintf(&b, "status:  %s\n", dashIfEmpty(string(s.Status)))
	fmt.Fprintf(&b, "rounds:  %d applied / %d max\n", len(s.Rounds), s.MaxRounds)
	fmt.Fprintf(&b, "seq:     %d\n", s.LatestSeq)

	if m.loadErr != nil {
		fmt.Fprintf(&b, "\nsnapshot load error: %v\n", m.loadErr)
	}
	if m.tailErr != nil {
		fmt.Fprintf(&b, "\ntailer error: %v\n", m.tailErr)
	}
	fmt.Fprintln(&b, "\npress q to quit")
	return b.String()
}

func dashIfEmpty(s string) string {
	if s == "" {
		return "—"
	}
	return s
}
