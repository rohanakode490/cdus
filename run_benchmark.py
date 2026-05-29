import socket
import json
import time
import sys

def send_ipc(socket_path, message):
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(socket_path)
        s.sendall(json.dumps(message).encode())
        data = s.recv(4096)
        if not data:
            return None
        return json.loads(data.decode())

def run_benchmark(sock1, sock2, node_id2):
    print(f"Triggering benchmark from {sock1} to {node_id2}")
    send_ipc(sock1, {"StartBenchmark": {"node_id": node_id2}})
    
    start_time = time.time()
    last_bytes = 0
    
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s:
        s.connect(sock2)
        s.sendall(json.dumps("ListenEvents").encode())
        print("Listening for progress events...")
        
        while True:
            data = s.recv(4096)
            if not data:
                break
            for line in data.decode().strip().split('\n'):
                if not line: continue
                event = json.loads(line)
                if "FileProgress" in event:
                    prog = event["FileProgress"]
                    if "Progress" in prog:
                        p = prog["Progress"]
                        bytes_conf = p["bytes_confirmed"]
                        total = p["total_bytes"]
                        elapsed = time.time() - start_time
                        if elapsed > 0:
                            speed = bytes_conf / elapsed / (1024*1024)
                            print(f"\rProgress: {bytes_conf/(1024*1024):.1f}MB / {total/(1024*1024):.1f}MB ({speed:.1f} MB/s)", end="")
                        last_bytes = bytes_conf
                    elif "Complete" in prog:
                        elapsed = time.time() - start_time
                        print(f"\nBenchmark Complete! Total time: {elapsed:.2f}s, Avg speed: {1024/elapsed:.2f} MB/s")
                        return

if __name__ == "__main__":
    run_benchmark("/tmp/cdus1.sock", "/tmp/cdus2.sock", "12D3KooWNWtGLu6XNgyB1TDRP6BMpzSqv483BZHWQxfGn3RCiNvt")
