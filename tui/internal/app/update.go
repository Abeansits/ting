package app

import (
	"time"

	tea "github.com/charmbracelet/bubbletea"

	"github.com/Abeansits/ting/tui/internal/data"
	"github.com/Abeansits/ting/tui/internal/model"
)

type tailerEventMsg struct{ Event model.Event }

type tailerErrMsg struct{ Err error }

type tailerClosedMsg struct{}

type tickMsg time.Time

type refreshedMsg struct {
	State *model.State
	Err   error
}

const tickInterval = 150 * time.Millisecond

func (m *Model) Init() tea.Cmd {
	go m.tailer.Run(m.ctx)
	m.ticking = true
	return tea.Batch(
		waitForEvent(m.tailer.Events()),
		waitForTailerErr(m.tailer.Errors()),
		tickCmd(),
	)
}

func (m *Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		return m.handleKey(msg)

	case tea.WindowSizeMsg:
		return m, nil

	case tickMsg:
		m.now = time.Time(msg)
		if !m.animating() {
			m.ticking = false
			return m, nil
		}
		return m, tickCmd()

	case tailerEventMsg:
		if err := m.state.Apply(msg.Event); err != nil {
			m.tailErr = err
		}
		return m, tea.Batch(waitForEvent(m.tailer.Events()), m.resumeTick())

	case tailerErrMsg:
		m.tailErr = msg.Err
		return m, waitForTailerErr(m.tailer.Errors())

	case tailerClosedMsg:
		return m, nil

	case refreshedMsg:
		if msg.Err != nil {
			m.loadErr = msg.Err
			return m, nil
		}
		if msg.State != nil && msg.State.LatestSeq > m.state.LatestSeq {
			m.state = msg.State
			m.loadErr = nil
		}
		return m, m.resumeTick()
	}
	return m, nil
}

func (m *Model) handleKey(msg tea.KeyMsg) (tea.Model, tea.Cmd) {
	switch msg.String() {
	case "q", "ctrl+c", "esc":
		m.Close()
		return m, tea.Quit
	case "?":
		m.showHelp = !m.showHelp
		return m, nil
	case "r":
		return m, refreshCmd(m.forumDir)
	case "up", "k":
		m.focusPrevRound()
		return m, nil
	case "down", "j":
		m.focusNextRound()
		return m, nil
	case "0":
		m.focusedRound = 0
		return m, nil
	}

	if r := msg.Runes; len(r) == 1 && r[0] >= '1' && r[0] <= '9' {
		m.jumpToRound(int(r[0] - '0'))
	}
	return m, nil
}

// resumeTick restarts the animation loop after a state event if the forum has
// transitioned back to in-progress. No-op if already ticking.
func (m *Model) resumeTick() tea.Cmd {
	if m.ticking || !m.animating() {
		return nil
	}
	m.ticking = true
	return tickCmd()
}

func (m *Model) focusPrevRound() {
	last := maxRoundNumber(m.state)
	if last == 0 {
		return
	}
	current := m.focusedRound
	if current == 0 {
		current = last
	}
	if current > 1 {
		m.focusedRound = current - 1
	}
}

func (m *Model) focusNextRound() {
	last := maxRoundNumber(m.state)
	if last == 0 {
		return
	}
	if m.focusedRound == 0 || m.focusedRound >= last {
		m.focusedRound = 0
		return
	}
	m.focusedRound++
}

func maxRoundNumber(s *model.State) int {
	var max uint32
	for _, r := range s.Rounds {
		if r.Round > max {
			max = r.Round
		}
	}
	return int(max)
}

func (m *Model) jumpToRound(n int) {
	for _, r := range m.state.Rounds {
		if int(r.Round) == n {
			m.focusedRound = n
			return
		}
	}
}

func tickCmd() tea.Cmd {
	return tea.Tick(tickInterval, func(t time.Time) tea.Msg { return tickMsg(t) })
}

func refreshCmd(forumDir string) tea.Cmd {
	return func() tea.Msg {
		s, err := data.LoadState(forumDir)
		return refreshedMsg{State: s, Err: err}
	}
}

func waitForEvent(ch <-chan model.Event) tea.Cmd {
	return func() tea.Msg {
		ev, ok := <-ch
		if !ok {
			return tailerClosedMsg{}
		}
		return tailerEventMsg{Event: ev}
	}
}

func waitForTailerErr(ch <-chan error) tea.Cmd {
	return func() tea.Msg {
		err, ok := <-ch
		if !ok {
			return nil
		}
		return tailerErrMsg{Err: err}
	}
}
