import socket
import json
import time
import subprocess
import os
import signal
import threading

def send_ipc(socket_path, message):
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.connect(socket_path)
            s.sendall(json.dumps(message).encode())
            data = s.recv(4096)
            if not data: return None
            return json.loads(data.decode())
    except Exception as e:
        return None

def start_agent(id, port, socket_path):
    data_dir = os.path.abspath(f"agent_data_{id}")
    if not os.path.exists(data_dir):
        os.makedirs(data_dir)
    
    cmd = ["./target/debug/cdus-agent", "--port", str(port), "--socket", socket_path, "--data-dir", data_dir]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True
    )
    for _ in range(20):
        if os.path.exists(socket_path):
            return proc
        time.sleep(0.5)
    return proc

def cleanup():
    for f in ["/tmp/agent1.sock", "/tmp/agent2.sock"]:
        if os.path.exists(f):
            os.remove(f)
    subprocess.run(["pkill", "-f", "cdus-agent"])

try:
    cleanup()
    print("Starting Agent 1...")
    a1 = start_agent(1, 5101, "/tmp/agent1.sock")
    if a1:
        print("Agent 1 socket appeared!")
        res = send_ipc("/tmp/agent1.sock", "Ping")
        print(f"Ping response: {res}")
    else:
        print("Agent 1 failed to start socket.")

finally:
    cleanup()
