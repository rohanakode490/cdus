import socket
import json
import time
import subprocess
import os
import signal

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
        print(f"IPC Error ({socket_path}): {e}")
        return None

def get_node_id(socket_path):
    # We can get node_id by connecting and listening for logs or checking state
    # But simpler: just start the agent and it prints it. 
    # For this test, we'll assume we can get it from 'GetDiscovered' or similar.
    # Actually, let's just use 'GetPairedDevices' after pairing.
    pass

def cleanup():
    subprocess.run(["killall", "-9", "cdus-agent"], stderr=subprocess.DEVNULL)
    for f in ["/tmp/cdus1.sock", "/tmp/cdus2.sock"]:
        if os.path.exists(f): os.remove(f)
    subprocess.run(["rm", "-rf", "/tmp/cdus1", "/tmp/cdus2"])
    os.makedirs("/tmp/cdus1", exist_ok=True)
    os.makedirs("/tmp/cdus2", exist_ok=True)

def start_agents():
    print("Starting agents...")
    a1 = subprocess.Popen(["./target/debug/cdus-agent", "--port", "5200", "--socket", "/tmp/cdus1.sock", "--data-dir", "/tmp/cdus1"], stdout=open("/tmp/agent1.log", "w"), stderr=subprocess.STDOUT)
    a2 = subprocess.Popen(["./target/debug/cdus-agent", "--port", "5201", "--socket", "/tmp/cdus2.sock", "--data-dir", "/tmp/cdus2"], stdout=open("/tmp/agent2.log", "w"), stderr=subprocess.STDOUT)
    time.sleep(2)
    return a1, a2

def test_resume():
    cleanup()
    
    # Create 200MB file
    file_path = os.path.abspath("test_data.bin")
    if not os.path.exists(file_path):
        print("Creating 200MB test file...")
        subprocess.run(["dd", "if=/dev/urandom", "of=" + file_path, "bs=1M", "count=200"])

    a1, a2 = start_agents()

    print("Pairing agents...")
    send_ipc("/tmp/cdus1.sock", {"PairWithIp": {"ip": "127.0.0.1", "port": 5201}})
    time.sleep(1)
    send_ipc("/tmp/cdus1.sock", {"ConfirmPairing": True})
    send_ipc("/tmp/cdus2.sock", {"ConfirmPairing": True})
    time.sleep(1)

    # Get Agent 2 Node ID
    paired = send_ipc("/tmp/cdus1.sock", "GetPairedDevices")
    if not paired or len(paired["PairedDevicesResponse"]) == 0:
        print("Pairing failed")
        return
    
    node_id2 = paired["PairedDevicesResponse"][0][0]
    print(f"Agent 2 Node ID: {node_id2}")

    print("Initiating transfer with CRASH TRIGGER at 100MB...")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s2:
        s2.connect("/tmp/cdus2.sock")
        s2.settimeout(0.5) # Don't block forever
        s2.sendall(json.dumps("ListenEvents").encode())
        
        send_ipc("/tmp/cdus1.sock", {"SendFile": {"node_id": node_id2, "path": file_path}})
        
        transfer_id = None
        while True:
            try:
                data = s2.recv(4096)
                if not data: break
                lines = data.decode().strip().split('\n')
                for line in lines:
                    if not line: continue
                    try:
                        event = json.loads(line)
                    except: continue
                    
                    if "FileProgress" in event:
                        prog = event["FileProgress"]
                        if "IncomingRequest" in prog:
                            transfer_id = prog["IncomingRequest"]["transfer_id"]
                            print(f"Detected incoming transfer: {transfer_id}")
                            
                            # Set crash trigger on SENDER (Agent 1)
                            print(f"Setting crash trigger for {transfer_id} at 100MB...")
                            res = send_ipc("/tmp/cdus1.sock", {"SetCrashTrigger": {"transfer_id": transfer_id, "offset": 100 * 1024 * 1024}})
                            print(f"SetCrashTrigger response: {res}")
                            
                            # Accept on Agent 2
                            send_ipc("/tmp/cdus2.sock", {"AcceptFileTransfer": {"transfer_id": transfer_id}})
                            print("Transfer accepted. Waiting for crash...")
                        
                        elif "Progress" in prog:
                            bc = prog["Progress"]["bytes_confirmed"]
                            print(f"\rProgress: {bc/(1024*1024):.1f}MB", end="", flush=True)
            except socket.timeout:
                pass
            
            if a1.poll() is not None:
                print(f"\nAgent 1 EXITED with code {a1.returncode}")
                if a1.returncode == 42:
                    print("Agent 1 CRASHED as expected!")
                    break
                else:
                    print("Agent 1 exited UNEXPECTEDLY or finished too early.")
                    return None
            time.sleep(0.1)

    print("Restarting Agent 1...")
    a1 = subprocess.Popen(["./target/debug/cdus-agent", "--port", "5200", "--socket", "/tmp/cdus1.sock", "--data-dir", "/tmp/cdus1"], stdout=open("/tmp/agent1_resume.log", "w"), stderr=subprocess.STDOUT)
    time.sleep(2)
    
    print(f"Requesting RESUME for {transfer_id}...")
    send_ipc("/tmp/cdus1.sock", {"ResumeFileTransfer": {"transfer_id": transfer_id}})
    
    print("Waiting for completion...")
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as s2:
        s2.connect("/tmp/cdus2.sock")
        s2.sendall(json.dumps("ListenEvents").encode())
        while True:
            data = s2.recv(4096)
            if not data: break
            lines = data.decode().strip().split('\n')
            for line in lines:
                if not line: continue
                try:
                    event = json.loads(line)
                except: continue
                
                if "FileProgress" in event:
                    prog = event["FileProgress"]
                    if "IncomingRequest" in prog:
                        tid = prog["IncomingRequest"]["transfer_id"]
                        print(f"Detected incoming resume request: {tid}")
                        send_ipc("/tmp/cdus2.sock", {"AcceptFileTransfer": {"transfer_id": tid}})
                    elif "Progress" in prog:
                        bc = prog["Progress"]["bytes_confirmed"]
                        print(f"\rResume Progress: {bc/(1024*1024):.1f}MB", end="", flush=True)
                    elif "Complete" in prog:
                        print("\nTransfer COMPLETE!")
                        dest = prog["Complete"]["dest_path"]
                        return dest
                    elif "Failed" in prog:
                        print(f"\nTransfer FAILED: {prog['Failed']['reason']}")
                        return None
            time.sleep(0.1)

if __name__ == "__main__":
    try:
        dest = test_resume()
        if dest:
            print(f"Verifying integrity: {dest}")
            # Run sha256sum on both
            s1 = subprocess.check_output(["sha256sum", "test_data.bin"]).split()[0]
            s2 = subprocess.check_output(["sha256sum", dest]).split()[0]
            if s1 == s2:
                print("SUCCESS: Hashes match!")
            else:
                print(f"FAILURE: Hash mismatch! {s1} != {s2}")
    finally:
        cleanup()
