package app

import (
	tea "github.com/charmbracelet/bubbletea"

	"github.com/Abeansits/ting/tui/internal/model"
)

type tailerEventMsg struct{ Event model.Event }

type tailerErrMsg struct{ Err error }

type tailerClosedMsg struct{}

// Init starts the tailer goroutine and the channel-receive commands.
func (m *Model) Init() tea.Cmd {
	go m.tailer.Run(m.ctx)
	return tea.Batch(
		waitForEvent(m.tailer.Events()),
		waitForTailerErr(m.tailer.Errors()),
	)
}

func (m *Model) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "q", "ctrl+c", "esc":
			m.Close()
			return m, tea.Quit
		}

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
	}
	return m, nil
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
