// Package kernelcb provides a small circuit breaker for the Rust kernel gRPC client.
package kernelcb

import (
	"errors"
	"os"
	"strconv"
	"sync"
	"time"
)

// ErrOpen is returned by Execute when the circuit is open and the call is short-circuited.
var ErrOpen = errors.New("kernel grpc circuit open")

type state int

const (
	stateClosed state = iota
	stateOpen
	stateHalfOpen
)

// Breaker trips to open after MaxFailures consecutive errors, then rejects calls until OpenTimeout elapses.
// Set MaxFailures to 0 (via NewFromEnv and ORCHESTRATOR_KERNEL_CB_MAX_FAILURES=0) to disable.
type Breaker struct {
	mu sync.Mutex

	st       state
	failures int
	openedAt time.Time

	MaxFailures int
	OpenTimeout time.Duration
	disabled    bool
}

// New returns a breaker with the given thresholds.
func New(maxFailures int, openTimeout time.Duration) *Breaker {
	if maxFailures <= 0 {
		return &Breaker{disabled: true}
	}
	if openTimeout <= 0 {
		openTimeout = 30 * time.Second
	}
	return &Breaker{
		MaxFailures: maxFailures,
		OpenTimeout: openTimeout,
	}
}

// NewFromEnv uses ORCHESTRATOR_KERNEL_CB_MAX_FAILURES (default 5, 0 = disabled) and
// ORCHESTRATOR_KERNEL_CB_OPEN_SECONDS (default 30).
func NewFromEnv() *Breaker {
	maxF := 5
	if v := os.Getenv("ORCHESTRATOR_KERNEL_CB_MAX_FAILURES"); v != "" {
		if n, err := strconv.Atoi(v); err == nil {
			maxF = n
		}
	}
	openSec := 30
	if v := os.Getenv("ORCHESTRATOR_KERNEL_CB_OPEN_SECONDS"); v != "" {
		if n, err := strconv.Atoi(v); err == nil && n > 0 {
			openSec = n
		}
	}
	return New(maxF, time.Duration(openSec)*time.Second)
}

// Execute runs fn when the circuit allows. If the circuit is open, returns ErrOpen without calling fn.
// fn errors count toward opening; a successful fn resets the failure count (closed) or closes from half-open.
func (b *Breaker) Execute(fn func() error) error {
	if b.disabled {
		return fn()
	}

	b.mu.Lock()
	switch b.st {
	case stateOpen:
		if time.Since(b.openedAt) >= b.OpenTimeout {
			b.st = stateHalfOpen
			b.failures = 0
		} else {
			b.mu.Unlock()
			return ErrOpen
		}
	case stateHalfOpen, stateClosed:
	}
	b.mu.Unlock()

	err := fn()

	b.mu.Lock()
	defer b.mu.Unlock()

	switch b.st {
	case stateHalfOpen:
		if err != nil {
			b.st = stateOpen
			b.openedAt = time.Now()
		} else {
			b.st = stateClosed
			b.failures = 0
		}
	case stateClosed:
		if err != nil {
			b.failures++
			if b.failures >= b.MaxFailures {
				b.st = stateOpen
				b.openedAt = time.Now()
			}
		} else {
			b.failures = 0
		}
	}
	return err
}
