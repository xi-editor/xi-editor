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

import Foundation

class CoreConnection {

    var inHandle: FileHandle  // stdin of core process
    var recvBuf: Data
    var callback: (AnyObject) -> ()

    // RPC state
    var queue = DispatchQueue(label: "com.levien.xi.CoreConnection", attributes: [])
    var rpcIndex = 0
    var pending = Dictionary<Int, (Any?) -> ()>()

    init(path: String, callback: @escaping (Any) -> ()) {
        let task = Process()
        task.launchPath = path
        task.arguments = []
        let outPipe = Pipe()
        task.standardOutput = outPipe
        let inPipe = Pipe()
        task.standardInput = inPipe
        inHandle = inPipe.fileHandleForWriting
        recvBuf = Data()
        self.callback = callback
        outPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            self.recvHandler(data)
        }
        task.launch()
    }

    func recvHandler(_ data: Data) {
        if data.count == 0 {
            print("eof")
            return
        }
        let scanStart = recvBuf.count
        recvBuf.append(data)
        let recvBufLen = recvBuf.count
        recvBuf.withUnsafeMutableBytes { (recvBufBytes: UnsafeMutablePointer<UInt8>) -> Void in
            var i = 0
            for j in scanStart..<recvBufLen {
                // TODO: using memchr would probably be faster
                if recvBufBytes[j] == UInt8(ascii:"\n") {
                    let dataPacket = recvBuf.subdata(in: i ..< j + 1)
                    handleRaw(dataPacket)
                    i = j + 1
                }
            }
            if i < recvBufLen {
                memmove(recvBufBytes, recvBufBytes + i, recvBufLen - i)
            }
            recvBuf.count = recvBufLen - i
        }
    }

    func sendJson(_ json: Any) {
        do {
            let data = try JSONSerialization.data(withJSONObject: json, options: [])
            let mutdata = NSMutableData()
            mutdata.append(data)
            let nl = [0x0a as UInt8]
            mutdata.append(nl, length: 1)
            inHandle.write(mutdata as Data)
        } catch _ {
            print("error serializing to json")
        }
    }

    func handleRaw(_ data: Data) {
        do {
            let json = try JSONSerialization.jsonObject(with: data, options: .allowFragments)
            //print("got \(json)")
            if !handleRpcResponse(json as AnyObject) {
                callback(json as AnyObject)
            }
        } catch _ {
            print("json error")
        }
    }

    func sendRpcAsync(_ method: String, params: Any, callback: ((Any?) -> ())? = nil) {
        var index = Int()
        var req = ["method": method, "params": params] as [String : Any]
        if let callback = callback {
            queue.sync {
                req["id"] = self.rpcIndex
                index = self.rpcIndex
                self.rpcIndex += 1
                self.pending[index] = callback
            }
        }
        sendJson(req as Any)
    }

    // send RPC synchronously, blocking until return
    func sendRpc(_ method: String, params: Any) -> Any? {
        let semaphore = DispatchSemaphore(value: 0)
        var result: Any? = nil
        sendRpcAsync(method, params: params) { r in
            result = r
            semaphore.signal()
        }
        semaphore.wait(timeout: DispatchTime.distantFuture)
        return result
    }

    func handleRpcResponse(_ response: Any) -> Bool {
        if let resp = response as? [String: Any], let index = resp["id"] as? Int {
            var callback: ((Any?) -> ())? = nil
            let result = resp["result"]
            queue.sync {
                callback = self.pending.removeValue(forKey: index)
            }
            callback?(result)
            return true;
        } else {
            return false;
        }
    }
}
