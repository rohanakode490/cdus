package hub

import (
	"context"
	"encoding/json"
	"log/slog"
	"os"
	"testing"
	"time"

	"cdus-relay/internal/domain"
)

type mockStore struct{}

func (m *mockStore) RegisterDevice(ctx context.Context, device *domain.Device) error { return nil }
func (m *mockStore) GetDevice(ctx context.Context, uuid string) (*domain.Device, error) {
	return nil, nil
}
func (m *mockStore) RevokeDevice(ctx context.Context, uuid string) error { return nil }
func (m *mockStore) IsDeviceRevoked(ctx context.Context, uuid string) (bool, error) {
	return false, nil
}
func (m *mockStore) Close() error                      { return nil }
func (m *mockStore) Ping(ctx context.Context) error    { return nil }

func TestHub_Run(t *testing.T) {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	ms := &mockStore{}
	h := NewHub(ms, logger)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()

	go h.Run(ctx)

	client1 := &Client{
		hub:  h,
		uuid: "client-1",
		send: make(chan []byte, 10),
	}
	client2 := &Client{
		hub:  h,
		uuid: "client-2",
		send: make(chan []byte, 10),
	}

	// Test Register
	h.register <- client1
	h.register <- client2

	// Give it a moment to process
	time.Sleep(10 * time.Millisecond)

	h.mu.RLock()
	if len(h.clients) != 2 {
		t.Errorf("expected 2 clients, got %d", len(h.clients))
	}
	h.mu.RUnlock()

	// Test Broadcast
	msg := domain.SignalMessage{
		SourceUUID: "client-1",
		TargetUUID: "client-2",
		Payload:    []byte("hello"),
	}
	h.broadcast <- msg

	select {
	case received := <-client2.send:
		var signal domain.SignalMessage
		_ = json.Unmarshal(received, &signal)
		if string(signal.Payload) != "hello" {
			t.Errorf("expected 'hello', got %s", string(signal.Payload))
		}
	case <-time.After(100 * time.Millisecond):
		t.Error("timed out waiting for broadcast")
	}

	// Test Unregister
	h.unregister <- client1
	time.Sleep(10 * time.Millisecond)

	h.mu.RLock()
	if len(h.clients) != 1 {
		t.Errorf("expected 1 client, got %d", len(h.clients))
	}
	h.mu.RUnlock()
}
