import socket
import json
import time
import sys
import subprocess
import os

def send_ipc(socket_path, message):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(socket_path)
        s.sendall(json.dumps(message).encode())
        data = s.recv(4096)
        if not data: return None
        return json.loads(data.decode())

def test_resume():
    # Create 512MB file
    if not os.path.exists("dummy.bin"):
        print("Creating 512MB dummy file...")
        subprocess.run(["dd", "if=/dev/urandom", "of=dummy.bin", "bs=1M", "count=512"])

    sock1 = "/tmp/cdus1.sock"
    sock2 = "/tmp/cdus2.sock"
    node_id2 = "12D3KooWNWtGLu6XNgyB1TDRP6BMpzSqv483BZHWQxfGn3RCiNvt"

    print("Starting transfer...")
    resp = send_ipc(sock1, {"SendFile": {"node_id": node_id2, "path": os.path.abspath("dummy.bin")}})
    print(f"SendFile response: {resp}")
    transfer_id = resp["FileTransferProgress"]["transfer_id"]

    # Wait for it to start and reach ~100MB
    print("Waiting for ~100MB...")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(sock2)
        s.sendall(json.dumps("ListenEvents").encode())
        while True:
            data = s.recv(4096)
            if not data: break
            lines = data.decode().strip().split('\n')
            for line in lines:
                if not line: continue
                event = json.loads(line)
                if "FileProgress" in event:
                    prog = event["FileProgress"]
                    if "IncomingRequest" in prog:
                        tid = prog["IncomingRequest"]["transfer_id"]
                        print(f"Accepting transfer {tid}")
                        send_ipc(sock2, {"AcceptFileTransfer": {"file_hash": tid}})
                    elif "Progress" in prog:
                        p = prog["Progress"]
                        bc = p["bytes_confirmed"]
                        print(f"\rProgress: {bc/(1024*1024):.1f}MB", end="")
                        if bc > 100 * 1024 * 1024:
                            print("\nReached 100MB, KILLING AGENT 2!")
                            subprocess.run(["killall", "cdus-agent"])
                            return transfer_id

if __name__ == "__main__":
    tid = test_resume()
    print(f"Killed. Waiting 2 seconds...")
    time.sleep(2)
    print("Restarting agents...")
    # Start agents again
    subprocess.Popen(["./target/debug/cdus-agent", "--port", "5200", "--socket", "/tmp/cdus1.sock", "--data-dir", "/tmp/cdus1", "--relay-url", "http://localhost:8080"], stdout=open("/tmp/agent1_res.log", "w"), stderr=subprocess.STDOUT)
    subprocess.Popen(["./target/debug/cdus-agent", "--port", "5201", "--socket", "/tmp/cdus2.sock", "--data-dir", "/tmp/cdus2", "--relay-url", "http://localhost:8080"], stdout=open("/tmp/agent2_res.log", "w"), stderr=subprocess.STDOUT)
    
    time.sleep(2)
    print("Pairing...")
    send_ipc("/tmp/cdus1.sock", {"PairWithIp": {"ip": "127.0.0.1", "port": 5201}})
    time.sleep(1)
    send_ipc("/tmp/cdus1.sock", {"ConfirmPairing": True})
    send_ipc("/tmp/cdus2.sock", {"ConfirmPairing": True})
    
    print(f"Resuming transfer {tid}...")
    # To resume, we just send the file again. It should use the same transfer_id if the file path and hash are same?
    # Actually, SendFile generates a NEW transfer_id.
    # To truly test resume, we might need a "ResumeTransfer" IPC command or it should auto-detect same file.
    # Current SendFile logic:
    # let transfer_id = Uuid::new_v4().to_string();
    # So it's always new.
    
    # Wait, the receiver side uses transfer_id (which is currently just the hash in some places, or a UUID).
    # Let's check how transfer_id is generated.
    
    print("Resume test needs manual check of bytes_confirmed in DB.")
