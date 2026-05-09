package server

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"log/slog"
	"net/http"
	"net/http/httptest"
	"os"
	"testing"
	"time"

	"cdus-relay/internal/domain"
	"cdus-relay/internal/hub"
)

type mockStore struct {
	devices   map[string]*domain.Device
	revoked   map[string]bool
	pingError error
	regError  error
}

func newMockStore() *mockStore {
	return &mockStore{
		devices: make(map[string]*domain.Device),
		revoked: make(map[string]bool),
	}
}

func (m *mockStore) RegisterDevice(ctx context.Context, device *domain.Device) error {
	if m.regError != nil {
		return m.regError
	}
	m.devices[device.UUID] = device
	return nil
}

func (m *mockStore) GetDevice(ctx context.Context, uuid string) (*domain.Device, error) {
	return m.devices[uuid], nil
}

func (m *mockStore) RevokeDevice(ctx context.Context, uuid string) error {
	m.revoked[uuid] = true
	return nil
}

func (m *mockStore) IsDeviceRevoked(ctx context.Context, uuid string) (bool, error) {
	return m.revoked[uuid], nil
}

func (m *mockStore) Ping(ctx context.Context) error {
	return m.pingError
}

func (m *mockStore) Close() error { return nil }

func TestHandleRegister(t *testing.T) {
	tests := []struct {
		name       string
		reqBody    interface{}
		regError   error
		expectedOk int
	}{
		{
			name: "success",
			reqBody: registerRequest{
				UUID:      "device-1",
				PublicKey: "key-1",
			},
			expectedOk: http.StatusCreated,
		},
		{
			name:       "invalid-json",
			reqBody:    "not-json",
			expectedOk: http.StatusBadRequest,
		},
		{
			name: "missing-uuid",
			reqBody: registerRequest{
				PublicKey: "key-1",
			},
			expectedOk: http.StatusBadRequest,
		},
		{
			name: "db-error",
			reqBody: registerRequest{
				UUID:      "device-2",
				PublicKey: "key-2",
			},
			regError:   errors.New("db error"),
			expectedOk: http.StatusInternalServerError,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ms := newMockStore()
			ms.regError = tt.regError
			logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
			h := hub.NewHub(ms, logger)
			srv := NewServer(ms, h, logger)

			var body []byte
			if s, ok := tt.reqBody.(string); ok {
				body = []byte(s)
			} else {
				body, _ = json.Marshal(tt.reqBody)
			}

			req, _ := http.NewRequest("POST", "/v1/register", bytes.NewBuffer(body))
			rr := httptest.NewRecorder()

			srv.Routes().ServeHTTP(rr, req)

			if rr.Code != tt.expectedOk {
				t.Errorf("expected status %d, got %d", tt.expectedOk, rr.Code)
			}
		})
	}
}

func TestHandleCreateToken(t *testing.T) {
	tests := []struct {
		name       string
		uuid       string
		revoked    bool
		expectedOk int
	}{
		{
			name:       "success",
			uuid:       "device-1",
			expectedOk: http.StatusOK,
		},
		{
			name:       "revoked",
			uuid:       "device-1",
			revoked:    true,
			expectedOk: http.StatusUnauthorized,
		},
		{
			name:       "unregistered",
			uuid:       "device-unknown",
			expectedOk: http.StatusOK, // In current implementation, we only check revocation
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ms := newMockStore()
			if tt.revoked {
				ms.revoked[tt.uuid] = true
			}
			logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
			h := hub.NewHub(ms, logger)
			srv := NewServer(ms, h, logger)

			reqBody, _ := json.Marshal(tokenRequest{UUID: tt.uuid})
			req, _ := http.NewRequest("POST", "/v1/pairing/token", bytes.NewBuffer(reqBody))
			rr := httptest.NewRecorder()

			srv.Routes().ServeHTTP(rr, req)

			if rr.Code != tt.expectedOk {
				t.Errorf("expected status %d, got %d", tt.expectedOk, rr.Code)
			}
		})
	}
}

func TestHandleHealthz(t *testing.T) {
	tests := []struct {
		name       string
		pingError  error
		expectedOk int
	}{
		{
			name:       "healthy",
			expectedOk: http.StatusOK,
		},
		{
			name:       "unhealthy",
			pingError:  errors.New("db unreachable"),
			expectedOk: http.StatusServiceUnavailable,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			ms := newMockStore()
			ms.pingError = tt.pingError
			logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))
			h := hub.NewHub(ms, logger)
			srv := NewServer(ms, h, logger)

			req, _ := http.NewRequest("GET", "/healthz", nil)
			rr := httptest.NewRecorder()

			srv.Routes().ServeHTTP(rr, req)

			if rr.Code != tt.expectedOk {
				t.Errorf("expected status %d, got %d", tt.expectedOk, rr.Code)
			}
		})
	}
}

func TestTokenManager_Cleanup(t *testing.T) {
	tm := NewTokenManager()
	
	// Create a token that expires in the past
	id := "expired-token"
	tm.mu.Lock()
	tm.tokens[id] = domain.Token{
		ID:        id,
		ExpiresAt: time.Now().Add(-1 * time.Minute),
	}
	tm.mu.Unlock()

	// Create a valid token
	validID := tm.Create("device-1")

	tm.Cleanup()

	if _, ok := tm.Get(id); ok {
		t.Error("expired token was not cleaned up")
	}

	if _, ok := tm.Get(validID); !ok {
		t.Error("valid token was cleaned up")
	}
}
