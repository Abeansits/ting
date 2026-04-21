package data

import (
	"bytes"
	"context"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"log"
	"os"
	"path/filepath"
	"sync"

	"github.com/fsnotify/fsnotify"

	"github.com/Abeansits/ting/tui/internal/model"
)

// Tailer streams events from dashboard-events.jsonl. It drains existing
// lines first, then watches the containing directory with fsnotify and
// re-drains on each write.
//
// Partial trailing bytes (no '\n' yet) are buffered and prepended to the
// next read. The Rust writer guarantees PIPE_BUF-atomic full-line appends
// per schemas/CONTRACT.md, so a mid-append read just gets less data, never
// a torn line.
//
// Malformed JSON and unknown event types are logged and skipped.
type Tailer struct {
	path      string
	events    chan model.Event
	errs      chan error
	ready     chan struct{}
	readyOnce sync.Once
	watcher   *fsnotify.Watcher
}

// NewTailer prepares the watcher. Call Run in a goroutine; cancel ctx to stop.
func NewTailer(path string) (*Tailer, error) {
	w, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	// Watch the parent directory so a freshly-created log file (Phase 1B
	// creates it on the first emitted event) still fires the first write.
	// Watching the file directly would miss the create.
	dir := filepath.Dir(path)
	if err := w.Add(dir); err != nil {
		_ = w.Close()
		return nil, fmt.Errorf("watch %s: %w", dir, err)
	}
	return &Tailer{
		path:    filepath.Clean(path),
		events:  make(chan model.Event, 64),
		errs:    make(chan error, 8),
		ready:   make(chan struct{}),
		watcher: w,
	}, nil
}

func (t *Tailer) Events() <-chan model.Event { return t.events }
func (t *Tailer) Errors() <-chan error       { return t.errs }

// Ready is closed once initial drain completes OR Run exits early (ctx
// cancelled, watcher closed). "Ready" therefore means "startup settled" —
// receivers should check Events/Errors afterward to decide the outcome.
func (t *Tailer) Ready() <-chan struct{} { return t.ready }

// Close stops the watcher. Run will exit when its events channel closes.
func (t *Tailer) Close() error { return t.watcher.Close() }

// Run drains the log, signals Ready, then pumps subsequent appends. Blocks
// until ctx is cancelled or the watcher closes. Closes Events and Errors on
// exit.
func (t *Tailer) Run(ctx context.Context) {
	defer close(t.events)
	defer close(t.errs)
	// Close ready on every exit so `<-Ready()` never hangs after an
	// early-cancel during the initial drain.
	defer t.signalReady()

	var (
		file *os.File
		buf  []byte
	)
	defer func() {
		if file != nil {
			_ = file.Close()
		}
	}()

	drain := func() bool {
		if file == nil {
			f, err := os.Open(t.path)
			if err != nil {
				if errors.Is(err, fs.ErrNotExist) {
					return true
				}
				return t.pushErr(ctx, err)
			}
			file = f
		}
		chunk := make([]byte, 4096)
		for {
			n, readErr := file.Read(chunk)
			if n > 0 {
				buf = append(buf, chunk[:n]...)
				for {
					i := bytes.IndexByte(buf, '\n')
					if i < 0 {
						break
					}
					line := buf[:i]
					buf = buf[i+1:]
					if len(bytes.TrimSpace(line)) == 0 {
						continue
					}
					evt, perr := model.ParseEvent(line)
					if perr != nil {
						if errors.Is(perr, model.ErrUnknownEventType) {
							log.Printf("tui/tailer: unknown event type %q at seq=%d — skipping", evt.Type, evt.Seq)
						} else {
							log.Printf("tui/tailer: skipping malformed line: %v", perr)
						}
						continue
					}
					select {
					case t.events <- evt:
					case <-ctx.Done():
						return false
					}
				}
			}
			if errors.Is(readErr, io.EOF) {
				return true
			}
			if readErr != nil {
				return t.pushErr(ctx, readErr)
			}
		}
	}

	if !drain() {
		return
	}
	t.signalReady()

	for {
		select {
		case <-ctx.Done():
			return
		case ev, ok := <-t.watcher.Events:
			if !ok {
				return
			}
			if filepath.Clean(ev.Name) != t.path {
				continue
			}
			switch {
			case ev.Op.Has(fsnotify.Write), ev.Op.Has(fsnotify.Create):
				if !drain() {
					return
				}
			case ev.Op.Has(fsnotify.Remove), ev.Op.Has(fsnotify.Rename):
				if file != nil {
					_ = file.Close()
					file = nil
				}
				buf = nil
			}
		case err, ok := <-t.watcher.Errors:
			if !ok {
				return
			}
			if !t.pushErr(ctx, err) {
				return
			}
		}
	}
}

// pushErr reports err on the errs channel; returns false if ctx cancelled
// mid-send so the caller can bail out of Run.
func (t *Tailer) pushErr(ctx context.Context, err error) bool {
	select {
	case t.errs <- err:
		return true
	case <-ctx.Done():
		return false
	}
}

func (t *Tailer) signalReady() {
	t.readyOnce.Do(func() { close(t.ready) })
}
