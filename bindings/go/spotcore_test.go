package spotcore

import (
	"errors"
	"sync"
	"testing"
)

func expectCode(t *testing.T, err error, code string) {
	t.Helper()
	var typed *Error
	if !errors.As(err, &typed) || typed.Code != code {
		t.Errorf("expected %s, got %v", code, err)
	}
}
func TestLifecycle(t *testing.T) {
	session, err := NewSession()
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()
	_, err = session.Account()
	expectCode(t, err, "not_authenticated")
	_, err = session.PollEvent()
	expectCode(t, err, "not_authenticated")
	_, err = session.Login("", SessionOptions{AppID: "go-test", DeviceName: "Binding test"}, nil)
	expectCode(t, err, "invalid_input")
	_, err = session.SearchTracks("\xff", 1, 0)
	expectCode(t, err, "invalid_input")
	var group sync.WaitGroup
	for i := 0; i < 12; i++ {
		group.Add(1)
		go func() { defer group.Done(); _, err := session.Account(); expectCode(t, err, "not_authenticated") }()
	}
	group.Wait()
	if err := session.Close(); err != nil {
		t.Fatal(err)
	}
	if err := session.Close(); err != nil {
		t.Fatal(err)
	}
	_, err = session.Account()
	expectCode(t, err, "closed")
}

func TestDomainOperations(t *testing.T) {
	session, err := NewSession()
	if err != nil {
		t.Fatal(err)
	}
	defer session.Close()
	inventory, err := session.Operations()
	if err != nil || len(inventory) < 170 {
		t.Fatalf("Missing inventory: %v", err)
	}
	_, err = session.Invoke("library.contains", map[string]any{"values": []string{"spotify:track:test"}})
	expectCode(t, err, "not_authenticated")
	_, err = session.Invoke("unknown.operation", nil)
	expectCode(t, err, "invalid_input")
}
