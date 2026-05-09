package main

import (
	"context"
	"flag"
	"log/slog"
	"net/http"
	"os"
	"os/signal"
	"syscall"
	"time"

	"cdus-relay/internal/hub"
	"cdus-relay/internal/server"
	"cdus-relay/internal/store"
)

func main() {
	port := flag.String("port", "8080", "HTTP port")
	dbPath := flag.String("db", "relay.db", "SQLite database path")
	flag.Parse()

	logger := slog.New(slog.NewJSONHandler(os.Stdout, nil))

	// Phase 2: Store
	s, err := store.NewSQLiteStore(*dbPath)
	if err != nil {
		logger.Error("failed to open store", "error", err)
		os.Exit(1)
	}
	defer s.Close()

	// Phase 4: Hub
	h := hub.NewHub(s, logger)
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	go h.Run(ctx)

	// Phase 3 & 4: Server
	srv := server.NewServer(s, h, logger)
	go srv.RunBackgroundTasks(ctx)

	httpServer := &http.Server{
		Addr:    ":" + *port,
		Handler: srv.Routes(),
	}

	// Phase 5: Graceful Shutdown
	go func() {
		logger.Info("starting server", "port", *port)
		if err := httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			logger.Error("failed to start server", "error", err)
			os.Exit(1)
		}
	}()

	// Wait for interrupt signal
	stop := make(chan os.Signal, 1)
	signal.Notify(stop, os.Interrupt, syscall.SIGTERM)
	<-stop

	logger.Info("shutting down server")

	shutdownCtx, shutdownCancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer shutdownCancel()

	if err := httpServer.Shutdown(shutdownCtx); err != nil {
		logger.Error("failed to shutdown gracefully", "error", err)
	}

	cancel() // Stop the hub
	logger.Info("server stopped")
}
