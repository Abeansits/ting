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

// Init starts the tailer goroutine, subscribes to its channels, and kicks
// off the animation tick.
func (m *Model) Init() tea.Cmd {
	go m.tailer.Run(m.ctx)
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
		m.width, m.height = msg.Width, msg.Height
		return m, nil

	case tickMsg:
		m.now = time.Time(msg)
		m.tick++
		return m, tickCmd()

	case tailerEventMsg:
		if err := m.state.Apply(msg.Event); err != nil {
			m.tailErr = err
		}
		return m, waitForEvent(m.tailer.Events())

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
		return m, nil
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
	}

	if r := msg.Runes; len(r) == 1 && r[0] >= '1' && r[0] <= '9' {
		m.jumpToRound(int(r[0] - '0'))
	}
	return m, nil
}

func (m *Model) focusPrevRound() {
	if len(m.state.Rounds) == 0 {
		return
	}
	current := m.focusedRound
	if current == 0 {
		current = int(m.state.Rounds[len(m.state.Rounds)-1].Round)
	}
	if current > 1 {
		m.focusedRound = current - 1
	}
}

func (m *Model) focusNextRound() {
	if len(m.state.Rounds) == 0 {
		return
	}
	last := int(m.state.Rounds[len(m.state.Rounds)-1].Round)
	current := m.focusedRound
	if current == 0 || current >= last {
		m.focusedRound = 0
		return
	}
	m.focusedRound = current + 1
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
