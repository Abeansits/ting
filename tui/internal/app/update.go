package app

import (
	tea "github.com/charmbracelet/bubbletea"

	"github.com/Abeansits/ting/tui/internal/model"
)

// tailerEventMsg carries one event from the tailer into the Bubble Tea loop.
type tailerEventMsg struct{ Event model.Event }

// tailerErrMsg surfaces a non-fatal tailer error (IO hiccup, watcher glitch)
// into the Update loop. 3A records the latest error; 3B can decide whether
// to render it.
type tailerErrMsg struct{ Err error }

// tailerClosedMsg fires when the tailer's events channel drains — either
// because Close was called or the watcher failed hard.
type tailerClosedMsg struct{}

// Init starts the tailer goroutine and returns the first wait-for-event
// command. Bubble Tea calls this once when the program starts.
func (m *Model) Init() tea.Cmd {
	go m.tailer.Run(m.ctx)
	return tea.Batch(
		waitForEvent(m.tailer.Events()),
		waitForTailerErr(m.tailer.Errors()),
	)
}

// Update folds one message into the model. Keeps the surface small for 3A;
// 3B can layer on key bindings, scrolling, and animation ticks.
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
