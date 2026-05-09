package store

import (
	"context"
	"database/sql"
	"fmt"
	"time"

	"cdus-relay/internal/domain"

	_ "modernc.org/sqlite"
)

type SQLiteStore struct {
	db *sql.DB
}

func NewSQLiteStore(dsn string) (*SQLiteStore, error) {
	db, err := sql.Open("sqlite", dsn)
	if err != nil {
		return nil, fmt.Errorf("failed to open database: %w", err)
	}

	// Enable WAL mode for better concurrency
	if _, err := db.Exec("PRAGMA journal_mode=WAL;"); err != nil {
		return nil, fmt.Errorf("failed to enable WAL: %w", err)
	}

	s := &SQLiteStore{db: db}
	if err := s.init(); err != nil {
		return nil, err
	}

	return s, nil
}

func (s *SQLiteStore) init() error {
	query := `
	CREATE TABLE IF NOT EXISTS devices (
		uuid TEXT PRIMARY KEY,
		public_key TEXT NOT NULL,
		created_at DATETIME NOT NULL
	);
	CREATE TABLE IF NOT EXISTS revocations (
		uuid TEXT PRIMARY KEY,
		revoked_at DATETIME NOT NULL
	);
	`
	_, err := s.db.Exec(query)
	return err
}

func (s *SQLiteStore) RegisterDevice(ctx context.Context, device *domain.Device) error {
	query := `INSERT OR REPLACE INTO devices (uuid, public_key, created_at) VALUES (?, ?, ?)`
	_, err := s.db.ExecContext(ctx, query, device.UUID, device.PublicKey, device.CreatedAt)
	return err
}

func (s *SQLiteStore) GetDevice(ctx context.Context, uuid string) (*domain.Device, error) {
	query := `SELECT uuid, public_key, created_at FROM devices WHERE uuid = ?`
	row := s.db.QueryRowContext(ctx, query, uuid)

	var d domain.Device
	if err := row.Scan(&d.UUID, &d.PublicKey, &d.CreatedAt); err != nil {
		if err == sql.ErrNoRows {
			return nil, nil
		}
		return nil, err
	}
	return &d, nil
}

func (s *SQLiteStore) RevokeDevice(ctx context.Context, uuid string) error {
	query := `INSERT OR REPLACE INTO revocations (uuid, revoked_at) VALUES (?, ?)`
	_, err := s.db.ExecContext(ctx, query, uuid, time.Now())
	return err
}

func (s *SQLiteStore) IsDeviceRevoked(ctx context.Context, uuid string) (bool, error) {
	query := `SELECT 1 FROM revocations WHERE uuid = ?`
	var exists int
	err := s.db.QueryRowContext(ctx, query, uuid).Scan(&exists)
	if err == sql.ErrNoRows {
		return false, nil
	}
	return err == nil, err
}

func (s *SQLiteStore) Close() error {
	return s.db.Close()
}

func (s *SQLiteStore) Ping(ctx context.Context) error {
	return s.db.PingContext(ctx)
}
