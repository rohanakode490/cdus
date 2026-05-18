import subprocess
import time
import os
import json
import socket
import hashlib
import shutil

# Paths
AGENT_BIN = "./target/debug/cdus-agent"
SOCKET1 = "/tmp/cdus-agent-1.sock"
SOCKET2 = "/tmp/cdus-agent-2.sock"
DATA1 = "/tmp/cdus-data-1"
DATA2 = "/tmp/cdus-data-2"
TEST_FILE = "/tmp/test_transfer.bin"
RECEIVED_FILE = "/tmp/cdus-data-2/test_transfer.bin"

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

def listen_until(socket_path, condition_fn, timeout=10):
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
                        print(f"Event on {socket_path}: {event}")
                        if condition_fn(event):
                            return event
                    except:
                        continue
            except socket.timeout:
                continue
    return None

def cleanup():
    for f in [SOCKET1, SOCKET2, TEST_FILE]:
        if os.path.exists(f): os.remove(f)
    for d in [DATA1, DATA2]:
        if os.path.exists(d): shutil.rmtree(d)
    os.makedirs(DATA1, exist_ok=True)
    os.makedirs(DATA2, exist_ok=True)

def main():
    cleanup()
    
    # Create test file
    content = os.urandom(1024 * 1024) # 1MB
    with open(TEST_FILE, "wb") as f:
        f.write(content)
    test_hash = hashlib.sha256(content).hexdigest()
    print(f"Created test file: {TEST_FILE} (Hash: {test_hash})")

    # Start Agent 1
    p1 = subprocess.Popen([AGENT_BIN, "--socket", SOCKET1, "--port", "5201", "--data-dir", DATA1])
    # Start Agent 2
    p2 = subprocess.Popen([AGENT_BIN, "--socket", SOCKET2, "--port", "5202", "--data-dir", DATA2])
    
    time.sleep(2) # Wait for start

    try:
        # Get Node IDs
        resp1 = send_ipc(SOCKET1, {"GetState": {"key": "node_id"}})
        id1 = resp1["StateResponse"]
        resp2 = send_ipc(SOCKET2, {"GetState": {"key": "node_id"}})
        id2 = resp2["StateResponse"]
        print(f"Agent 1 ID: {id1}")
        print(f"Agent 2 ID: {id2}")

        # Step 1: Pair
        print("Initiating pairing via IP...")
        resp = send_ipc(SOCKET1, {"PairWithIp": {"ip": "127.0.0.1", "port": 5202}})
        print(f"Pairing init response: {resp}")
        
        # Step 2: Wait for both to see active pairing
        print("Waiting for active pairing state...")
        for i in range(15):
            s1 = send_ipc(SOCKET1, "GetPairingStatus")
            s2 = send_ipc(SOCKET2, "GetPairingStatus")
            
            if not s1 or not s2:
                print(f"Loop {i}: IPC failed")
                time.sleep(1)
                continue

            status1 = s1.get("PairingStatusResponse")
            status2 = s2.get("PairingStatusResponse")

            if not status1 or not status2:
                print(f"Loop {i}: Missing PairingStatusResponse. S1: {s1}, S2: {s2}")
                time.sleep(1)
                continue

            if status1["active"] and status2["active"]:
                print(f"Pairing active! PIN: {status1['pin']}")
                break
            
            print(f"Loop {i}: S1 active={status1['active']}, S2 active={status2['active']}")
            time.sleep(1)
        else:
            print("Pairing failed to become active")
            return

        print("Confirming pairing on both sides...")
        send_ipc(SOCKET1, {"ConfirmPairing": True})
        send_ipc(SOCKET2, {"ConfirmPairing": True})
        
        # Step 3: Send File
        print(f"Sending file {TEST_FILE} to {id2}...")
        send_ipc(SOCKET1, {"SendFile": {"node_id": id2, "path": TEST_FILE}})
        
        # Step 4: Listen for incoming on Agent 2
        print("Agent 2 waiting for incoming request...")
        req_event = listen_until(SOCKET2, lambda e: "IncomingFileRequest" in e)
        if not req_event:
            print("Failed to receive file request")
            return

        file_hash = req_event["IncomingFileRequest"]["manifest"]["file_hash"]
        print(f"Accepting transfer for hash {file_hash}...")
        send_ipc(SOCKET2, {"AcceptFileTransfer": {"file_hash": file_hash}})
        
        # Step 5: Wait for completion
        print("Waiting for transfer completion...")
        completion_event = listen_until(SOCKET2, lambda e: e.get("TransferProgress", {}).get("status") == "Completed")
        
        if completion_event:
            print("Transfer completed!")
            # Check file
            received_path = os.path.join(DATA2, "test_transfer.bin")
            if os.path.exists(received_path):
                with open(received_path, "rb") as f:
                    received_content = f.read()
                received_hash = hashlib.sha256(received_content).hexdigest()
                if received_hash == test_hash:
                    print("VERIFICATION SUCCESS: Hashes match!")
                else:
                    print(f"VERIFICATION FAILURE: Hash mismatch! Got {received_hash}")
            else:
                print(f"VERIFICATION FAILURE: File not found at {received_path}")
        else:
            print("Transfer timed out or failed")

    finally:
        p1.terminate()
        p2.terminate()
        p1.wait()
        p2.wait()
        # cleanup()

if __name__ == "__main__":
    main()
