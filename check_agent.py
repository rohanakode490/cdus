import socket
import os
import time

socket_path = os.environ.get("CDUS_AGENT_SOCKET", "/tmp/cdus-agent.sock")
print(f"Checking socket: {socket_path}")

for i in range(5):
    if not os.path.exists(socket_path):
        print(f"[{i}] Socket does not exist yet...")
        time.sleep(1)
        continue
    
    try:
        with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
            s.settimeout(2)
            s.connect(socket_path)
            print("Successfully connected to socket!")
            # Try to send a Ping
            msg = '"Ping"'
            s.sendall(msg.encode())
            resp = s.recv(1024)
            print(f"Response: {resp.decode()}")
            break
    except Exception as e:
        print(f"[{i}] Connection failed: {e}")
        time.sleep(1)
