package app

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/charmbracelet/lipgloss"
	"github.com/muesli/termenv"

	"github.com/Abeansits/ting/tui/internal/data"
	"github.com/Abeansits/ting/tui/internal/model"
)

// Force plain output so tests assert on literal substrings. lipgloss's
// color profile is otherwise auto-detected from the TERM env and can still
// emit ANSI even under `go test`.
func init() { lipgloss.SetColorProfile(termenv.Ascii) }

func TestView_EmptyState(t *testing.T) {
	m := newTestModel(t, t.TempDir())
	out := m.View()

	for _, want := range []string{
		"TING",
		"ROUNDS",
		"METRICS",
		"CONVERGENCE",
		"SYNTHESIS",
		"q quit",
		"? help",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in empty view:\n%s", want, out)
		}
	}
}

func TestView_FixtureState(t *testing.T) {
	m := newFixtureModel(t)
	out := m.View()

	for _, want := range []string{
		"Is the sky falling?",
		"ting-2026-04-19-abcd1234",
		"codex",
		"gemini",
		"claude",
		"Feasibility",
		"Dissent",
		"Round 1/3",
		"round progress",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q in fixture view:\n%s", want, out)
		}
	}
}

func TestView_DissentAxisMarked(t *testing.T) {
	m := newFixtureModel(t)
	out := m.View()
	// We don't assert on ANSI codes (profile is off), but we require the
	// axis label to render so the reverse-style is attached to something.
	if !strings.Contains(out, "Dissent") {
		t.Fatalf("Dissent label missing in:\n%s", out)
	}
}

func TestView_ConvergenceAndMetricScores(t *testing.T) {
	m := newFixtureModel(t)

	// Inject a metric_scores + convergence event for round 1 so the
	// renderer exercises bar + sparkline + history paths.
	applyJSON(t, m.state, 5, "metric_scores", map[string]any{
		"round": 1,
		"scores": []map[string]any{
			{"metric_id": "feasibility", "score": 7.0},
			{"metric_id": "dissent_axis", "score": 3.5},
		},
	})
	applyJSON(t, m.state, 6, "convergence", map[string]any{
		"round": 1,
		"score": 6.0,
	})

	out := m.View()
	for _, want := range []string{
		"7.0 / 10",
		"3.5 / 10",
		"6.0 / 10",
	} {
		if !strings.Contains(out, want) {
			t.Errorf("missing %q after scoring events:\n%s", want, out)
		}
	}
}

func TestView_HelpOverlay(t *testing.T) {
	m := newTestModel(t, t.TempDir())
	m.showHelp = true
	out := m.View()
	if !strings.Contains(out, "keyboard") {
		t.Errorf("help overlay missing:\n%s", out)
	}
	if strings.Contains(out, "ROUNDS") {
		t.Errorf("main body leaked into help overlay:\n%s", out)
	}
}

func TestView_FocusMarker(t *testing.T) {
	m := newFixtureModel(t)
	// Add a second round so focus navigation has something to do.
	applyJSON(t, m.state, 5, "round_started", map[string]any{
		"round": 2,
		"stage": "proposal",
	})
	m.focusedRound = 2
	out := m.View()
	// The focus marker is "▸ " — we check the glyph appears *somewhere*
	// other than the topic heading (which always uses it).
	marks := strings.Count(out, "▸")
	if marks < 2 {
		t.Errorf("expected focus marker in rounds table (>=2 triangles), got %d:\n%s", marks, out)
	}
}

func TestRenderBar_Bounds(t *testing.T) {
	cases := []struct {
		name         string
		value, scale float64
		width        int
	}{
		{"zero", 0, 10, 10},
		{"half", 5, 10, 10},
		{"full", 10, 10, 10},
		{"over", 20, 10, 8},
		{"negative", -3, 10, 8},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			got := renderBar(c.value, c.scale, c.width)
			// Rough length check: each cell is one printable rune.
			printable := []rune(stripStyle(got))
			if len(printable) != c.width {
				t.Errorf("width = %d, want %d (got %q)", len(printable), c.width, string(printable))
			}
		})
	}
}

func TestRenderSparkline_LengthMatchesMaxRounds(t *testing.T) {
	out := renderSparkline([]float64{1, 5, 9}, 10, 5)
	if runes := []rune(stripStyle(out)); len(runes) != 5 {
		t.Errorf("sparkline len = %d, want 5 (%q)", len(runes), string(runes))
	}
}

func TestFormatElapsed(t *testing.T) {
	start := time.Date(2026, 4, 21, 12, 0, 0, 0, time.UTC)
	s := &model.State{Created: start, Status: model.StatusInProgress}
	got := formatElapsed(s, start.Add(75*time.Second))
	if got != "1m 15s" {
		t.Errorf("got %q", got)
	}

	s2 := &model.State{}
	if formatElapsed(s2, start) != "—" {
		t.Errorf("zero Created should render dash")
	}
}

func TestRefreshedMsg_ClearsLoadErrOnSuccess(t *testing.T) {
	m := newTestModel(t, t.TempDir())
	m.loadErr = errDummy("first read failed")

	// A successful refresh with equal-or-lower seq must still clear the
	// prior error so a transient failure doesn't stick on screen forever.
	equalState := &model.State{LatestSeq: m.state.LatestSeq}
	updated, _ := m.Update(refreshedMsg{State: equalState})
	m = updated.(*Model)
	if m.loadErr != nil {
		t.Fatalf("loadErr should clear on successful refresh, got %v", m.loadErr)
	}
}

type errDummy string

func (e errDummy) Error() string { return string(e) }

// --- helpers ---

func newTestModel(t *testing.T, dir string) *Model {
	t.Helper()
	m, err := New(context.Background(), dir)
	if err != nil {
		t.Fatalf("New: %v", err)
	}
	t.Cleanup(m.Close)
	return m
}

func newFixtureModel(t *testing.T) *Model {
	t.Helper()
	dir := t.TempDir()
	for _, name := range []string{data.StateFilename, data.EventLogFilename} {
		body, err := os.ReadFile(filepath.Join("..", "..", "testdata", "sample-forum", name))
		if err != nil {
			t.Fatalf("read fixture %s: %v", name, err)
		}
		if err := os.WriteFile(filepath.Join(dir, name), body, 0o644); err != nil {
			t.Fatalf("write fixture %s: %v", name, err)
		}
	}
	return newTestModel(t, dir)
}

func applyJSON(t *testing.T, s *model.State, seq uint64, kind string, payload any) {
	t.Helper()
	raw, err := json.Marshal(payload)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	ev := model.Event{
		Version:   1,
		Seq:       seq,
		ForumID:   s.ForumID,
		Timestamp: time.Now(),
		Type:      model.EventType(kind),
		Payload:   raw,
	}
	if err := s.Apply(ev); err != nil {
		t.Fatalf("apply %s: %v", kind, err)
	}
}

// stripStyle removes ANSI escape sequences so width assertions remain stable
// even if a future lipgloss version emits codes despite the disabled profile.
func stripStyle(s string) string {
	var b strings.Builder
	inEsc := false
	for _, r := range s {
		switch {
		case r == 0x1b:
			inEsc = true
		case inEsc && (r == 'm' || r == 'K'):
			inEsc = false
		case inEsc:
			// swallow
		default:
			b.WriteRune(r)
		}
	}
	return b.String()
}
