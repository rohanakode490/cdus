package main

import (
	"bytes"
	"encoding/json"
	"flag"
	"fmt"
	"log"
	"math/rand"
	"net/http"
	"sync"
	"sync/atomic"
	"time"

	"github.com/gorilla/websocket"
)

type registerRequest struct {
	UUID      string `json:"uuid"`
	PublicKey string `json:"public_key"`
}

type SignalMessage struct {
	SourceUUID string `json:"source_uuid"`
	TargetUUID string `json:"target_uuid"`
	Payload    []byte `json:"payload"`
}

func main() {
	relayURL := flag.String("url", "http://localhost:8080", "Base URL of the relay server")
	numDevices := flag.Int("devices", 50, "Number of concurrent devices to simulate")
	durationSecs := flag.Int("duration", 10, "Duration of the test in seconds")
	flag.Parse()

	log.Printf("Starting load test on %s with %d devices for %d seconds...", *relayURL, *numDevices, *durationSecs)

	uuids := make([]string, *numDevices)
	for i := 0; i < *numDevices; i++ {
		uuids[i] = fmt.Sprintf("simulated-device-%d-%d", i, time.Now().UnixNano())
	}

	// 1. Register all devices via HTTP POST
	var wgRegister sync.WaitGroup
	var registrationErrors int64

	regStart := time.Now()
	for _, uuid := range uuids {
		wgRegister.Add(1)
		go func(id string) {
			defer wgRegister.Done()
			reqBody, _ := json.Marshal(registerRequest{
				UUID:      id,
				PublicKey: "simulated-public-key-data-content-placeholder",
			})
			resp, err := http.Post(fmt.Sprintf("%s/v1/register", *relayURL), "application/json", bytes.NewBuffer(reqBody))
			if err != nil {
				log.Printf("POST error for %s: %v", id, err)
				atomic.AddInt64(&registrationErrors, 1)
				return
			}
			defer resp.Body.Close()
			if resp.StatusCode != http.StatusOK && resp.StatusCode != http.StatusCreated {
				log.Printf("POST status %d for %s", resp.StatusCode, id)
				atomic.AddInt64(&registrationErrors, 1)
			}
		}(uuid)
	}
	wgRegister.Wait()
	regDuration := time.Since(regStart)

	log.Printf("Registration phase completed. Registered %d devices in %v. Errors: %d", *numDevices, regDuration, registrationErrors)
	if registrationErrors == int64(*numDevices) {
		log.Fatalf("All registrations failed. Is the server running at %s?", *relayURL)
	}

	// Determine WebSocket base URL
	wsBase := *relayURL
	if len(wsBase) > 7 && wsBase[:7] == "http://" {
		wsBase = "ws://" + wsBase[7:]
	} else if len(wsBase) > 8 && wsBase[:8] == "https://" {
		wsBase = "wss://" + wsBase[8:]
	} else {
		wsBase = "ws://" + wsBase
	}

	var wgConnections sync.WaitGroup
	var sentCount int64
	var recvCount int64
	var connErrors int64
	var wsErrors int64

	stopChan := make(chan struct{})

	// 2. Establish WebSocket signaling loops
	for i, uuid := range uuids {
		wgConnections.Add(1)
		go func(idx int, sourceID string) {
			defer wgConnections.Done()

			wsURL := fmt.Sprintf("%s/v1/signaling?uuid=%s", wsBase, sourceID)
			conn, _, err := websocket.DefaultDialer.Dial(wsURL, nil)
			if err != nil {
				atomic.AddInt64(&connErrors, 1)
				return
			}
			defer conn.Close()

			// Start receiver pump
			go func() {
				for {
					_, _, err := conn.ReadMessage()
					if err != nil {
						return
					}
					atomic.AddInt64(&recvCount, 1)
				}
			}()

			// Start sender pump
			ticker := time.NewTicker(200 * time.Millisecond)
			defer ticker.Stop()

			for {
				select {
				case <-stopChan:
					return
				case <-ticker.C:
					// Select target device
					targetIdx := rand.Intn(*numDevices)
					if targetIdx == idx {
						targetIdx = (targetIdx + 1) % *numDevices
					}
					targetID := uuids[targetIdx]

					msg := SignalMessage{
						SourceUUID: sourceID,
						TargetUUID: targetID,
						Payload:    []byte("benchmark-load-payload-data-hello-world"),
					}
					msgBytes, _ := json.Marshal(msg)
					err := conn.WriteMessage(websocket.TextMessage, msgBytes)
					if err != nil {
						atomic.AddInt64(&wsErrors, 1)
						return
					}
					atomic.AddInt64(&sentCount, 1)
				}
			}
		}(i, uuid)
	}

	time.Sleep(time.Duration(*durationSecs) * time.Second)
	close(stopChan)
	wgConnections.Wait()

	// Results Summary
	fmt.Printf("\n=== LOAD TEST RESULTS ===\n")
	fmt.Printf("Simulated Devices: %d\n", *numDevices)
	fmt.Printf("Test Duration: %d seconds\n", *durationSecs)
	fmt.Printf("Successful Connections: %d\n", int64(*numDevices)-connErrors)
	fmt.Printf("Connection Dial Errors: %d\n", connErrors)
	fmt.Printf("WebSocket Write Errors: %d\n", wsErrors)
	fmt.Printf("Total Messages Sent: %d\n", sentCount)
	fmt.Printf("Total Messages Received: %d\n", recvCount)
	fmt.Printf("Throughput (Sent): %.1f msg/sec\n", float64(sentCount)/float64(*durationSecs))
	fmt.Printf("Throughput (Recv): %.1f msg/sec\n", float64(recvCount)/float64(*durationSecs))
	fmt.Printf("=========================\n")
}
