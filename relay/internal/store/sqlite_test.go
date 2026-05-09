package store

import (
	"context"
	"os"
	"testing"
	"time"

	"cdus-relay/internal/domain"
)

func TestSQLiteStore(t *testing.T) {
	dbPath := "test.db"
	defer os.Remove(dbPath)

	s, err := NewSQLiteStore(dbPath)
	if err != nil {
		t.Fatalf("failed to create store: %v", err)
	}
	defer s.Close()

	ctx := context.Background()

	// Test Ping
	if err := s.Ping(ctx); err != nil {
		t.Errorf("expected ping to succeed, got %v", err)
	}

	// Test Register
	device := &domain.Device{
		UUID:      "device-1",
		PublicKey: "key-1",
		CreatedAt: time.Now(),
	}
	if err := s.RegisterDevice(ctx, device); err != nil {
		t.Errorf("failed to register device: %v", err)
	}

	// Test Get
	got, err := s.GetDevice(ctx, "device-1")
	if err != nil {
		t.Errorf("failed to get device: %v", err)
	}
	if got == nil || got.UUID != "device-1" {
		t.Errorf("expected device-1, got %v", got)
	}

	// Test Get Non-existent
	got, err = s.GetDevice(ctx, "non-existent")
	if err != nil {
		t.Errorf("failed to get non-existent device: %v", err)
	}
	if got != nil {
		t.Errorf("expected nil for non-existent device, got %v", got)
	}

	// Test Revocation
	revoked, err := s.IsDeviceRevoked(ctx, "device-1")
	if err != nil {
		t.Errorf("failed to check revocation: %v", err)
	}
	if revoked {
		t.Error("expected device-1 to NOT be revoked")
	}

	if err := s.RevokeDevice(ctx, "device-1"); err != nil {
		t.Errorf("failed to revoke device: %v", err)
	}

	revoked, err = s.IsDeviceRevoked(ctx, "device-1")
	if err != nil {
		t.Errorf("failed to check revocation after revoke: %v", err)
	}
	if !revoked {
		t.Error("expected device-1 to be revoked")
	}
}
