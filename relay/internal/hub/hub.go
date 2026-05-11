package hub

import (
	"context"
	"encoding/json"
	"log/slog"
	"net/http"
	"sync"
	"time"

	"cdus-relay/internal/domain"
	"cdus-relay/internal/store"

	"github.com/gorilla/websocket"
)

const (
	writeWait      = 10 * time.Second
	pongWait       = 60 * time.Second
	pingPeriod     = (pongWait * 9) / 10
	maxMessageSize = 4096 // 4KB as per design
)

var upgrader = websocket.Upgrader{
	ReadBufferSize:  1024,
	WriteBufferSize: 1024,
	CheckOrigin: func(r *http.Request) bool {
		return true // In production, refine this
	},
}

type Client struct {
	hub  *Hub
	conn *websocket.Conn
	send chan []byte
	uuid string
}

func (c *Client) readPump() {
	defer func() {
		c.hub.unregister <- c
		c.conn.Close()
	}()

	c.conn.SetReadLimit(maxMessageSize)
	c.conn.SetReadDeadline(time.Now().Add(pongWait))
	c.conn.SetPongHandler(func(string) error {
		c.conn.SetReadDeadline(time.Now().Add(pongWait))
		return nil
	})

	for {
		_, message, err := c.conn.ReadMessage()
		if err != nil {
			if websocket.IsUnexpectedCloseError(err, websocket.CloseGoingAway, websocket.CloseAbnormalClosure) {
				c.hub.logger.Error("websocket error", "error", err, "uuid", c.uuid)
			}
			break
		}

		var signal domain.SignalMessage
		if err := json.Unmarshal(message, &signal); err != nil {
			continue
		}

		// Security: override source_uuid to prevent spoofing
		signal.SourceUUID = c.uuid
		c.hub.broadcast <- signal
	}
}

func (c *Client) writePump() {
	ticker := time.NewTicker(pingPeriod)
	defer func() {
		ticker.Stop()
		c.conn.Close()
	}()

	for {
		select {
		case message, ok := <-c.send:
			c.conn.SetWriteDeadline(time.Now().Add(writeWait))
			if !ok {
				c.conn.WriteMessage(websocket.CloseMessage, []byte{})
				return
			}

			w, err := c.conn.NextWriter(websocket.TextMessage)
			if err != nil {
				return
			}
			w.Write(message)

			if err := w.Close(); err != nil {
				return
			}
		case <-ticker.C:
			c.conn.SetWriteDeadline(time.Now().Add(writeWait))
			if err := c.conn.WriteMessage(websocket.PingMessage, nil); err != nil {
				return
			}
		}
	}
}

type Hub struct {
	clients             map[string]*Client
	broadcast           chan domain.SignalMessage
	broadcastRevocation chan domain.RevocationEvent
	register            chan *Client
	unregister          chan *Client
	store               store.Store
	logger              *slog.Logger
	mu                  sync.RWMutex
}

func NewHub(store store.Store, logger *slog.Logger) *Hub {
	return &Hub{
		broadcast:           make(chan domain.SignalMessage, 256),
		broadcastRevocation: make(chan domain.RevocationEvent, 256),
		register:            make(chan *Client),
		unregister:          make(chan *Client),
		clients:             make(map[string]*Client),
		store:               store,
		logger:              logger,
	}
}

func (h *Hub) Run(ctx context.Context) {
	for {
		select {
		case client := <-h.register:
			h.mu.Lock()
			h.clients[client.uuid] = client
			h.mu.Unlock()
		case client := <-h.unregister:
			h.mu.Lock()
			if _, ok := h.clients[client.uuid]; ok {
				delete(h.clients, client.uuid)
				close(client.send)
			}
			h.mu.Unlock()
		case signal := <-h.broadcast:
			h.mu.RLock()
			target, ok := h.clients[signal.TargetUUID]
			h.mu.RUnlock()

			if ok {
				data, _ := json.Marshal(signal)
				select {
				case target.send <- data:
				default:
					close(target.send)
					h.mu.Lock()
					delete(h.clients, signal.TargetUUID)
					h.mu.Unlock()
				}
			}
		case rev := <-h.broadcastRevocation:
			h.mu.RLock()
			data, _ := json.Marshal(rev)
			for _, client := range h.clients {
				select {
				case client.send <- data:
				default:
					// If client buffer is full, they'll miss it for now.
					// In a real system, we'd ensure reliable delivery.
				}
			}
			h.mu.RUnlock()
		case <-ctx.Done():
			return
		}
	}
}

func (h *Hub) BroadcastRevocation(uuid string) {
	h.broadcastRevocation <- domain.RevocationEvent{RevokedUUID: uuid}
}

func (h *Hub) DisconnectClient(uuid string) {
	h.mu.RLock()
	client, ok := h.clients[uuid]
	h.mu.RUnlock()

	if ok {
		h.unregister <- client
	}
}

func (h *Hub) ServeWs(w http.ResponseWriter, r *http.Request, deviceUUID string) {
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		h.logger.Error("failed to upgrade websocket", "error", err)
		return
	}

	client := &Client{
		hub:  h,
		conn: conn,
		send: make(chan []byte, 256),
		uuid: deviceUUID,
	}
	h.register <- client

	go client.writePump()
	go client.readPump()
}
