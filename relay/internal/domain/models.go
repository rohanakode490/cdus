package domain

import "time"

// Device represents a registered device in the system.
type Device struct {
	UUID      string    `json:"uuid"`
	PublicKey string    `json:"public_key"`
	CreatedAt time.Time `json:"created_at"`
}

// Token represents a short-lived pairing token.
type Token struct {
	ID         string    `json:"id"`
	DeviceUUID string    `json:"device_uuid"`
	ExpiresAt  time.Time `json:"expires_at"`
}

// SignalMessage represents an encrypted signaling message routed between peers.
type SignalMessage struct {
	SourceUUID string `json:"source_uuid"`
	TargetUUID string `json:"target_uuid"`
	Payload    []byte `json:"payload"` // Opaque E2EE payload
}

// RevocationEvent is broadcast to all clients when a device is revoked.
type RevocationEvent struct {
	RevokedUUID string `json:"revoked_uuid"`
}
