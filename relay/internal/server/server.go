package server

import (
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"sync"
	"time"

	"cdus-relay/internal/domain"
	"cdus-relay/internal/hub"
	"cdus-relay/internal/store"

	"github.com/google/uuid"
)

type TokenManager struct {
	mu     sync.RWMutex
	tokens map[string]domain.Token
}

func NewTokenManager() *TokenManager {
	return &TokenManager{
		tokens: make(map[string]domain.Token),
	}
}

func (tm *TokenManager) Create(deviceUUID string) string {
	id := uuid.New().String()[:8] // Short 8-char token
	tm.mu.Lock()
	defer tm.mu.Unlock()
	tm.tokens[id] = domain.Token{
		ID:         id,
		DeviceUUID: deviceUUID,
		ExpiresAt:  time.Now().Add(60 * time.Second),
	}
	return id
}

func (tm *TokenManager) Get(id string) (string, bool) {
	tm.mu.RLock()
	defer tm.mu.RUnlock()
	t, ok := tm.tokens[id]
	if !ok || time.Now().After(t.ExpiresAt) {
		return "", false
	}
	return t.DeviceUUID, true
}

// Cleanup removes expired tokens.
func (tm *TokenManager) Cleanup() {
	tm.mu.Lock()
	defer tm.mu.Unlock()
	now := time.Now()
	for id, t := range tm.tokens {
		if now.After(t.ExpiresAt) {
			delete(tm.tokens, id)
		}
	}
}

type Server struct {
	store  store.Store
	tokens *TokenManager
	hub    *hub.Hub
	logger *slog.Logger
}

func NewServer(store store.Store, hub *hub.Hub, logger *slog.Logger) *Server {
	return &Server{
		store:  store,
		tokens: NewTokenManager(),
		hub:    hub,
		logger: logger,
	}
}

func (s *Server) Routes() http.Handler {
	mux := http.NewServeMux()
	mux.HandleFunc("GET /healthz", s.handleHealthz)
	mux.HandleFunc("POST /v1/register", s.handleRegister)
	mux.HandleFunc("POST /v1/pairing/token", s.handleCreateToken)
	mux.HandleFunc("GET /v1/signaling", s.handleSignaling)
	return mux
}

func (s *Server) handleHealthz(w http.ResponseWriter, r *http.Request) {
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()

	if err := s.store.Ping(ctx); err != nil {
		s.logger.Error("health check failed", "error", err)
		w.WriteHeader(http.StatusServiceUnavailable)
		json.NewEncoder(w).Encode(map[string]string{"status": "unhealthy", "error": err.Error()})
		return
	}

	w.WriteHeader(http.StatusOK)
	json.NewEncoder(w).Encode(map[string]string{"status": "healthy"})
}

func (s *Server) RunBackgroundTasks(ctx context.Context) {
	ticker := time.NewTicker(1 * time.Minute)
	defer ticker.Stop()

	for {
		select {
		case <-ticker.C:
			s.tokens.Cleanup()
		case <-ctx.Done():
			return
		}
	}
}

type registerRequest struct {
	UUID      string `json:"uuid"`
	PublicKey string `json:"public_key"`
}

func (s *Server) handleRegister(w http.ResponseWriter, r *http.Request) {
	var req registerRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "invalid request", http.StatusBadRequest)
		return
	}

	if req.UUID == "" || req.PublicKey == "" {
		http.Error(w, "missing uuid or public_key", http.StatusBadRequest)
		return
	}

	device := &domain.Device{
		UUID:      req.UUID,
		PublicKey: req.PublicKey,
		CreatedAt: time.Now(),
	}

	if err := s.store.RegisterDevice(r.Context(), device); err != nil {
		s.logger.Error("failed to register device", "error", err, "uuid", req.UUID)
		http.Error(w, "internal error", http.StatusInternalServerError)
		return
	}

	w.WriteHeader(http.StatusCreated)
}

type tokenRequest struct {
	UUID string `json:"uuid"`
}

type tokenResponse struct {
	Token string `json:"token"`
}

func (s *Server) handleCreateToken(w http.ResponseWriter, r *http.Request) {
	var req tokenRequest
	if err := json.NewDecoder(r.Body).Decode(&req); err != nil {
		http.Error(w, "invalid request", http.StatusBadRequest)
		return
	}

	// Verify device exists and is not revoked
	revoked, err := s.store.IsDeviceRevoked(r.Context(), req.UUID)
	if err != nil || revoked {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	token := s.tokens.Create(req.UUID)
	json.NewEncoder(w).Encode(tokenResponse{Token: token})
}

func (s *Server) handleSignaling(w http.ResponseWriter, r *http.Request) {
	deviceUUID := r.URL.Query().Get("uuid")
	if deviceUUID == "" {
		http.Error(w, "missing uuid", http.StatusBadRequest)
		return
	}

	// In a real scenario, we'd verify a session token or Noise handshake state here.
	// For MVP, we trust the UUID if it's registered and not revoked.
	revoked, err := s.store.IsDeviceRevoked(r.Context(), deviceUUID)
	if err != nil || revoked {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}

	s.hub.ServeWs(w, r, deviceUUID)
}
