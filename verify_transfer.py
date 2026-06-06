import subprocess
import time
import os
import json
import socket
import hashlib
import shutil
import sys

# Paths
AGENT_BIN = "./target/debug/cdus-agent"
SOCKET1 = "/tmp/cdus-agent-1.sock"
SOCKET2 = "/tmp/cdus-agent-2.sock"
DATA1 = "/tmp/cdus-data-1"
DATA2 = "/tmp/cdus-data-2"
DOWNLOAD_DIR = "/tmp/cdus-downloads-2"
TEST_FILE = "/tmp/test_transfer.bin"

def send_ipc(socket_path, message):
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(socket_path)
            s.sendall(json.dumps(message).encode())
            data = s.recv(4096)
            if not data:
                return None
            return json.loads(data.decode())
    except Exception as e:
        print(f"IPC Error on {socket_path}: {e}")
        return None

def listen_events(socket_path, handler_fn, timeout=10):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(socket_path)
        s.sendall(json.dumps("ListenEvents").encode())
        s.settimeout(1.0)
        start_time = time.time()
        while time.time() - start_time < timeout:
            try:
                data = s.recv(4096)
                if not data:
                    continue
                for line in data.decode().strip().split('\n'):
                    if not line: continue
                    try:
                        event = json.loads(line)
                        if handler_fn(event):
                            return
                    except:
                        continue
            except socket.timeout:
                continue

def cleanup():
    # Kill existing agents
    subprocess.run(["killall", "-9", "cdus-agent"], stderr=subprocess.DEVNULL)
    
    for f in [SOCKET1, SOCKET2, TEST_FILE]:
        if os.path.exists(f): os.remove(f)
    for d in [DATA1, DATA2, DOWNLOAD_DIR]:
        if os.path.exists(d): shutil.rmtree(d)
    os.makedirs(DATA1, exist_ok=True)
    os.makedirs(DATA2, exist_ok=True)
    os.makedirs(DOWNLOAD_DIR, exist_ok=True)

def main():
    if not os.path.exists(AGENT_BIN):
        print(f"Error: {AGENT_BIN} not found. Please build the project first.")
        sys.exit(1)

    cleanup()
    
    # Create test file (5MB)
    file_size = 5 * 1024 * 1024
    content = os.urandom(file_size)
    with open(TEST_FILE, "wb") as f:
        f.write(content)
    test_hash = hashlib.sha256(content).hexdigest()
    print(f"Created test file: {TEST_FILE} (Size: 5MB, Hash: {test_hash})")

    # Start Agents
    print("Starting agents...")
    p1 = subprocess.Popen([AGENT_BIN, "--socket", SOCKET1, "--port", "5201", "--data-dir", DATA1], 
                          stdout=open("/tmp/agent1.log", "w"), stderr=subprocess.STDOUT)
    p2 = subprocess.Popen([AGENT_BIN, "--socket", SOCKET2, "--port", "5202", "--data-dir", DATA2, "--download-dir", DOWNLOAD_DIR], 
                          stdout=open("/tmp/agent2.log", "w"), stderr=subprocess.STDOUT)
    
    time.sleep(2) # Wait for start

    try:
        # Get Node IDs
        id1 = send_ipc(SOCKET1, {"GetState": {"key": "node_id"}})["StateResponse"]
        id2 = send_ipc(SOCKET2, {"GetState": {"key": "node_id"}})["StateResponse"]
        print(f"Agent 1 ID: {id1}")
        print(f"Agent 2 ID: {id2}")

        # Step 1: Pair
        print("Initiating pairing via IP...")
        send_ipc(SOCKET1, {"PairWithIp": {"ip": "127.0.0.1", "port": 5202}})
        
        # Step 2: Wait for both to see active pairing and confirm
        print("Waiting for active pairing state...")
        paired = False
        for i in range(15):
            s1 = send_ipc(SOCKET1, "GetPairingStatus")["PairingStatusResponse"]
            s2 = send_ipc(SOCKET2, "GetPairingStatus")["PairingStatusResponse"]
            
            if s1["active"] and s2["active"]:
                print(f"Pairing active! PIN: {s1['pin']}")
                send_ipc(SOCKET1, {"ConfirmPairing": True})
                send_ipc(SOCKET2, {"ConfirmPairing": True})
                paired = True
                break
            time.sleep(1)
        
        if not paired:
            print("Pairing failed to become active")
            return

        # Step 3: Send File
        print(f"Sending file to {id2}...")
        send_ipc(SOCKET1, {"SendFile": {"node_id": id2, "path": TEST_FILE}})
        
        # Step 4: Listen for incoming on Agent 2 and accept
        print("Waiting for incoming request and completion...")
        state = {"received_path": None}

        def event_handler(event):
            if "FileProgress" in event:
                prog = event["FileProgress"]
                if "IncomingRequest" in prog:
                    transfer_id = prog["IncomingRequest"]["transfer_id"]
                    print(f"Accepting transfer: {transfer_id}")
                    send_ipc(SOCKET2, {"AcceptFileTransfer": {"transfer_id": transfer_id}})
                
                elif "Progress" in prog:
                    p = prog["Progress"]
                    bc = p["bytes_confirmed"]
                    total = p["total_bytes"]
                    pct = (bc / total) * 100
                    print(f"\rProgress: {bc/(1024*1024):.1f}MB / {total/(1024*1024):.1f}MB ({pct:.1f}%)", end="", flush=True)
                
                elif "Complete" in prog:
                    print("\nTransfer completed!")
                    state["received_path"] = prog["Complete"]["dest_path"]
                    return True
                
                elif "Failed" in prog:
                    print(f"\nTransfer failed: {prog['Failed']['reason']}")
                    return True
            return False

        listen_events(SOCKET2, event_handler, timeout=30)
        received_path = state["received_path"]

        if received_path:
            # Verify file
            if os.path.exists(received_path):
                with open(received_path, "rb") as f:
                    received_content = f.read()
                received_hash = hashlib.sha256(received_content).hexdigest()
                if received_hash == test_hash:
                    print(f"VERIFICATION SUCCESS: Hashes match! Received at: {received_path}")
                else:
                    print(f"VERIFICATION FAILURE: Hash mismatch! Got {received_hash}")
            else:
                print(f"VERIFICATION FAILURE: File not found at {received_path}")
        else:
            print("Transfer did not complete successfully")

    finally:
        p1.terminate()
        p2.terminate()
        p1.wait()
        p2.wait()

if __name__ == "__main__":
    main()
