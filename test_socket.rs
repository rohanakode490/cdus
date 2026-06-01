use interprocess::local_socket::LocalSocketListener;
use std::io::Read;

fn main() {
    let socket_name = "/tmp/test-cdus.sock";
    let _ = std::fs::remove_file(socket_name);
    let listener = LocalSocketListener::bind(socket_name).unwrap();
    println!("Bound to {}", socket_name);
    for stream in listener.incoming() {
        match stream {
            Ok(mut s) => {
                println!("Got connection!");
                let mut buf = vec![0u8; 10];
                s.read(&mut buf).ok();
                println!("Read: {:?}", buf);
            }
            Err(e) => println!("Err: {}", e),
        }
    }
}
