// Copyright 2016 Google Inc. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Main process for Fuchsia builds - uses fidl rather than stdin / stdout

extern crate magenta;
extern crate mxruntime;

extern crate apps_modular_services_application_service_provider;
extern crate apps_xi_editor_rust_interfaces;
use self::apps_modular_services_application_service_provider::{ServiceProvider, ServiceProvider_Stub};
use self::apps_xi_editor_rust_interfaces::{Json, Json_Stub};

use std::thread;
use std::io;
use std::io::{Read, Write};
use std::sync::Arc;

use self::magenta::{Channel, HandleBase, Socket, Status};
use self::magenta::{MX_SOCKET_READABLE, MX_SOCKET_PEER_CLOSED, MX_TIME_INFINITE};
use self::mxruntime::{HandleType, get_startup_handle};

use fidl::Server;

use serde_json;
use serde_json::Value;

use xi_rpc::{RpcLoop};

use MainState;

pub struct MySocket(Arc<Socket>);

fn status_to_io_err(_status: Status) -> io::Error {
    // TODO: better error mapping
    io::Error::new(io::ErrorKind::Other, "OS error")
}

impl io::Read for MySocket {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let wait_sigs = MX_SOCKET_READABLE | MX_SOCKET_PEER_CLOSED;
        match self.0.wait(wait_sigs, MX_TIME_INFINITE) {
            Ok(signals) => {
                if signals.contains(MX_SOCKET_PEER_CLOSED) {
                    return Ok(0)
                }
            }
            Err(status) => return Err(status_to_io_err(status))
        }
        self.0.read(Default::default(), buf).or_else(|status|
            if status == Status::ErrRemoteClosed {
                Ok(0)
            } else {
                Err(status_to_io_err(status))
            }
        )
    }
}

impl io::Write for MySocket {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(Default::default(), buf).map_err(|status|
            // TODO: handle case where socket is full (wait and retry)
            status_to_io_err(status)
        )
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn editor_main(sock: Socket) {
    let mut state = MainState::new();
    let arc_sock = Arc::new(sock);
    let my_in = io::BufReader::new(MySocket(arc_sock.clone()));
    let my_out = MySocket(arc_sock);
    let mut rpc_looper = RpcLoop::new(my_out);

    rpc_looper.mainloop(|| my_in, &mut state);
}

struct JsonServer(());

impl Json for JsonServer {
    fn connect_socket(&mut self, sock: Socket) {
        let _ = thread::spawn(move || editor_main(sock));
    }
}

impl Json_Stub for JsonServer {
    // Use default dispatching, but we could override it here.
}
impl_fidl_stub!(JsonServer : Json_Stub);

struct ServiceProviderServer(());

impl ServiceProvider for ServiceProviderServer {
    fn connect_to_service(&mut self, service_name: String, channel: Channel) {
        // TODO: should probably get service name from hello service metadata
        if service_name == "xi.Json" {
            let json_server = JsonServer(());
            let _ = Server::new(json_server, channel).spawn();
        } else {
            print_err!("unknown service name {}", service_name);
        }
    }
}

impl ServiceProvider_Stub for ServiceProviderServer {
    // Use default dispatching, but we could override it here.
}
impl_fidl_stub!(ServiceProviderServer : ServiceProvider_Stub);

pub fn fidl_main() {
    let startup_handle = get_startup_handle(HandleType::OutgoingServices)
        .expect("couldn't get outgoing services handle");
    let chan = Channel::from_handle(startup_handle);
    let my_server = ServiceProviderServer(());
    let server_thread = Server::new(my_server, chan).spawn();
    let _ = server_thread.join();
}
