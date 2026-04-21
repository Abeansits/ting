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

	"github.com/fsnotify/fsnotify"

	"github.com/Abeansits/ting/tui/internal/model"
)

// Tailer streams events from dashboard-events.jsonl. It drains the existing
// contents first, then watches the containing directory via fsnotify and
// re-drains on writes.
//
// Partial-line handling: a trailing chunk without a final '\n' is buffered
// and prepended to the next read. The Rust writer guarantees PIPE_BUF-atomic
// full-line appends (see schemas/CONTRACT.md), so a torn read here is always
// just "we called Read mid-append, which is fine — the rest will arrive".
//
// Unknown event types and malformed JSON are logged and skipped. Per the
// contract, the tailer must never fatal the consumer on bad data.
type Tailer struct {
	path    string
	events  chan model.Event
	errs    chan error
	watcher *fsnotify.Watcher
}

// NewTailer prepares the watcher but does not start draining. Call Run in a
// goroutine; cancel the context to stop.
func NewTailer(path string) (*Tailer, error) {
	w, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, fmt.Errorf("new watcher: %w", err)
	}
	// Watch the parent directory so a freshly-created log file (v0.4 Phase
	// 1B will create the file on the first emitted event) still fires the
	// first write event. Watching the file directly would miss the create.
	if err := w.Add(filepath.Dir(path)); err != nil {
		_ = w.Close()
		return nil, fmt.Errorf("watch %s: %w", filepath.Dir(path), err)
	}
	return &Tailer{
		path:    path,
		events:  make(chan model.Event, 64),
		errs:    make(chan error, 8),
		watcher: w,
	}, nil
}

func (t *Tailer) Events() <-chan model.Event { return t.events }
func (t *Tailer) Errors() <-chan error       { return t.errs }

// Close stops the watcher. Run will see its events channel close and exit.
func (t *Tailer) Close() error { return t.watcher.Close() }

// Run drains the log and then pumps appends. Blocks until ctx is cancelled
// or the watcher closes. Closes Events and Errors on exit.
func (t *Tailer) Run(ctx context.Context) {
	defer close(t.events)
	defer close(t.errs)

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

	for {
		select {
		case <-ctx.Done():
			return
		case ev, ok := <-t.watcher.Events:
			if !ok {
				return
			}
			if ev.Name != t.path {
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
				buf = buf[:0]
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

// pushErr sends err on the errs channel; returns false if ctx cancelled
// mid-send so the caller can bail out of Run.
func (t *Tailer) pushErr(ctx context.Context, err error) bool {
	select {
	case t.errs <- err:
		return true
	case <-ctx.Done():
		return false
	}
}
