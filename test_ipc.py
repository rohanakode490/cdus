import socket
import json
import sys
import time

def send_ipc(socket_path, message):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(socket_path)
        s.sendall(json.dumps(message).encode())
        data = s.recv(4096)
        if not data:
            return None
        return json.loads(data.decode())

def listen_events(socket_path, duration=5):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(socket_path)
        s.sendall(json.dumps("ListenEvents").encode())
        s.settimeout(duration)
        events = []
        start_time = time.time()
        while time.time() - start_time < duration:
            try:
                data = s.recv(4096)
                if not data:
                    break
                # Data might contain multiple JSON messages separated by newlines
                for line in data.decode().strip().split('\n'):
                    if line:
                        events.append(json.loads(line))
            except socket.timeout:
                break
        return events

if __name__ == "__main__":
    cmd = sys.argv[1]
    sock = sys.argv[2]
    
    if cmd == "ping":
        print(send_ipc(sock, "Ping"))
    elif cmd == "get_discovered":
        print(send_ipc(sock, "GetDiscovered"))
    elif cmd == "pair":
        node_id = sys.argv[3]
        print(send_ipc(sock, {"PairWith": {"node_id": node_id}}))
    elif cmd == "confirm":
        accepted = sys.argv[3].lower() == "true"
        print(send_ipc(sock, {"ConfirmPairing": accepted}))
    elif cmd == "get_status":
        print(send_ipc(sock, "GetPairingStatus"))
    elif cmd == "send_file":
        node_id = sys.argv[3]
        path = sys.argv[4]
        print(send_ipc(sock, {"SendFile": {"node_id": node_id, "path": path}}))
    elif cmd == "accept":
        file_hash = sys.argv[3]
        print(send_ipc(sock, {"AcceptFileTransfer": {"file_hash": file_hash}}))
    elif cmd == "listen":
        duration = int(sys.argv[3]) if len(sys.argv) > 3 else 5
        for event in listen_events(sock, duration):
            print(json.dumps(event))
