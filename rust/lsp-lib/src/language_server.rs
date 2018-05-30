use lsp_types::InitializeParams;
use url::Url;
use std::collections::HashMap;
use std::io::Write;
use Callback;
use std::process;
use serde_json;
use serde_json::{Value, to_value};
use jsonrpc_lite::{JsonRpc, Error, Id, Params};
use lsp_types::{ClientCapabilities};

pub struct LanguageServerClient {
    writer: Box<Write + Send>,
    pending: HashMap<usize, Callback>,
    next_id: usize,
}

fn prepare_lsp_json(msg: &Value) -> Result<String, serde_json::error::Error> {
    let request = serde_json::to_string(&msg)?;
    Ok(format!("Content-Length: {}\r\n\r\n{}", request.len(), request))
}

fn number_from_id(id: Option<&Id>) -> usize {
    let id = id.expect("response missing id field");
    let id = match id {
        &Id::Num(n) => n as u64,
        &Id::Str(ref s) => u64::from_str_radix(s, 10).expect("failed to convert string id to u64"),
        other => panic!("unexpected value for id field: {:?}", other),
    };

    id as usize
}

impl LanguageServerClient {
    pub fn new(writer: Box<Write+Send>) -> Self {
        LanguageServerClient {
            writer,
            pending: HashMap::new(),
            next_id: 1
        }
    }

    pub fn write(&mut self, msg: &str) {
        self.writer
            .write_all(msg.as_bytes())
            .expect("error writing to stdin");

        self.writer.flush().expect("error flushing child stdin");
    }

    pub fn handle_message(&mut self, message: &str) {

        let value = JsonRpc::parse(message).unwrap();
        eprintln!("Received: {:?}", value);

        match value {
            JsonRpc::Request(obj) => eprintln!("client received unexpected request: {:?}", obj),
            JsonRpc::Notification(obj) => eprintln!("recv notification: {:?}", obj),
            JsonRpc::Success(ref obj) => {
                let mut result = value.get_result().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                self.handle_response(id, result);
            }
            JsonRpc::Error(ref obj) => {
                let mut error = value.get_error().unwrap().to_owned();
                let id = number_from_id(value.get_id().as_ref());
                self.handle_error(id, error);
            }
        };
    }

    pub fn handle_response(&mut self, id: usize, result: Value) {
        let callback = self.pending.remove(&id).expect(&format!("id {} missing from request table", id));
        callback.call(Ok(result));
    }

    pub fn handle_error(&mut self, id: usize, error: Error) {
        let callback = self.pending.remove(&id).expect(&format!("id {} missing from request table", id));
        callback.call(Err(error));
    }

     pub fn send_request(&mut self, method: &str, params: Params, completion: Callback) {
        
        let request = JsonRpc::request_with_params(Id::Num(self.next_id as i64), method, params);
        
        self.pending.insert(self.next_id, completion);
        self.next_id += 1;

        self.send_rpc(to_value(&request).unwrap());
    }

    fn send_rpc(&mut self, value: Value) {
        let rpc = match prepare_lsp_json(&value) {
            Ok(r) => r,
            Err(err) => panic!("Encoding Error {:?}",err)
         };

        self.write(rpc.as_ref());
    }

    pub fn send_notification(&mut self, method: &str, params: Params) {
        let notification = JsonRpc::notification_with_params(method, params);
        self.send_rpc(to_value(&notification).unwrap());
    }

}

// impl LanguageServerClient {

//     pub fn send_initialize<F>(&mut self, root_uri: Option<Url>, on_init: Callback)
//     where
//         F: 'static + Send + FnOnce(Result<Value, Error>) -> (),
//     {
//         let client_capabilities = ClientCapabilities::default();

//         let init_params = InitializeParams {
//             process_id: Some(process::id() as u64),
//             root_uri: root_uri,
//             root_path: None,
//             initialization_options: None,
//             capabilities: client_capabilities,
//             trace: None,
//         };

//         let params = Params::from(serde_json::to_value(init_params).unwrap());

//         self.send_request("initialize", params, on_init);
//     }

//     pub fn send_did_open(&mut self) {
//         eprintln!("DID OPEN CALLED")
//     }

//     pub fn request_diagonostics(&mut self) {}
// }
