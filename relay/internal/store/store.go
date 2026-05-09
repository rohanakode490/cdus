package store

import (
	"cdus-relay/internal/domain"
	"context"
)

// Store defines the persistence layer for the signaling server.
type Store interface {
	// Device management
	RegisterDevice(ctx context.Context, device *domain.Device) error
	GetDevice(ctx context.Context, uuid string) (*domain.Device, error)

	// Revocation
	RevokeDevice(ctx context.Context, uuid string) error
	IsDeviceRevoked(ctx context.Context, uuid string) (bool, error)

	// Close closes the underlying storage.
	Close() error

	// Ping checks if the storage is reachable.
	Ping(ctx context.Context) error
}
