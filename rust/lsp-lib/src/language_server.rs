use std::collections::HashMap;
use std::io::Write;
use Callback;
use serde_json;
use serde_json::{Value, to_value};
use jsonrpc_lite::{JsonRpc, Error, Id, Params};

pub struct LanguageServer {
    writer: Box<Write + Send>,
    pending: HashMap<usize, Callback>,
    next_id: usize,
}

fn prepare_lsp_json(msg: &Value) -> Result<String, serde_json::error::Error> {
    let request = serde_json::to_string(&msg)?;
    Ok(format!("Content-Length: {}\r\n\r\n{}", request.len(), request))
}

impl LanguageServer {
    pub fn new(writer: Box<Write+Send>) -> Self {
        LanguageServer {
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


