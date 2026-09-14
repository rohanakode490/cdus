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

type mockStore struct {
	devices map[string]*domain.Device
	revoked map[string]bool
}

func newMockStore() *mockStore {
	return &mockStore{
		devices: make(map[string]*domain.Device),
		revoked: make(map[string]bool),
	}
}

func (m *mockStore) RegisterDevice(ctx context.Context, device *domain.Device) error {
	m.devices[device.UUID] = device
	return nil
}
func (m *mockStore) GetDevice(ctx context.Context, uuid string) (*domain.Device, error) {
	if dev, ok := m.devices[uuid]; ok {
		return dev, nil
	}
	return nil, nil
}
func (m *mockStore) RevokeDevice(ctx context.Context, uuid string) error {
	m.revoked[uuid] = true
	return nil
}
func (m *mockStore) IsDeviceRevoked(ctx context.Context, uuid string) (bool, error) {
	return m.revoked[uuid], nil
}
func (m *mockStore) Close() error                                  { return nil }
func (m *mockStore) Ping(ctx context.Context) error                { return nil }
func (m *mockStore) CountDevices(ctx context.Context) (int, error) { return len(m.devices), nil }
func (m *mockStore) SaveFeedback(ctx context.Context, deviceUUID string, content string, logs string) error {
	return nil
}
func (m *mockStore) SaveTelemetry(ctx context.Context, deviceUUID string, payload string) error {
	return nil
}

func assertClientCount(t *testing.T, h *Hub, expected int, timeout time.Duration) {
	t.Helper()
	deadline := time.Now().Add(timeout)
	for time.Now().Before(deadline) {
		h.mu.RLock()
		count := len(h.clients)
		h.mu.RUnlock()
		if count == expected {
			return
		}
		time.Sleep(1 * time.Millisecond)
	}
	h.mu.RLock()
	defer h.mu.RUnlock()
	t.Fatalf("expected %d clients within %v, got %d", expected, timeout, len(h.clients))
}

func TestHub_Run(t *testing.T) {
	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
	ms := newMockStore()
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

	h.register <- client1
	h.register <- client2
	assertClientCount(t, h, 2, 100*time.Millisecond)

	msg := domain.SignalMessage{
		SourceUUID: "client-1",
		TargetUUID: "client-2",
		Payload:    []byte("hello"),
	}
	h.broadcast <- msg

	select {
	case received := <-client2.send:
		var signal domain.SignalMessage
		if err := json.Unmarshal(received, &signal); err != nil {
			t.Fatalf("failed to decode signal message: %v", err)
		}
		if string(signal.Payload) != "hello" {
			t.Errorf("expected 'hello', got %s", string(signal.Payload))
		}
	case <-time.After(100 * time.Millisecond):
		t.Fatal("timed out waiting for broadcast")
	}

	h.BroadcastRevocation("client-1")
	select {
	case received := <-client2.send:
		var rev domain.RevocationEvent
		if err := json.Unmarshal(received, &rev); err != nil {
			t.Fatalf("failed to decode revocation event: %v", err)
		}
		if rev.RevokedUUID != "client-1" {
			t.Errorf("expected revoked UUID 'client-1', got %s", rev.RevokedUUID)
		}
	case <-time.After(100 * time.Millisecond):
		t.Fatal("timed out waiting for revocation broadcast")
	}

	h.DisconnectClient("client-1")
	assertClientCount(t, h, 1, 100*time.Millisecond)
}
