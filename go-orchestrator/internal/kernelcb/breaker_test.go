package kernelcb

import (
	"errors"
	"testing"
	"time"
)

func TestBreaker_Disabled(t *testing.T) {
	t.Parallel()
	b := New(0, time.Second)
	if err := b.Execute(func() error { return errors.New("x") }); err == nil || err.Error() != "x" {
		t.Fatalf("expected passthrough error, got %v", err)
	}
}

func TestBreaker_OpensAndRecovers(t *testing.T) {
	t.Parallel()
	b := New(3, 50*time.Millisecond)
	fail := errors.New("grpc down")

	for i := 0; i < 3; i++ {
		if err := b.Execute(func() error { return fail }); !errors.Is(err, fail) {
			t.Fatalf("iteration %d: want fail got %v", i, err)
		}
	}
	if err := b.Execute(func() error { return nil }); !errors.Is(err, ErrOpen) {
		t.Fatalf("want ErrOpen, got %v", err)
	}

	time.Sleep(60 * time.Millisecond)

	if err := b.Execute(func() error { return nil }); err != nil {
		t.Fatalf("half-open success: %v", err)
	}
	if err := b.Execute(func() error { return nil }); err != nil {
		t.Fatalf("closed success: %v", err)
	}
}
