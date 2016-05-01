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

    var inHandle: NSFileHandle  // stdin of core process
    var sizeBuf: NSMutableData
    var recvBuf: NSMutableData
    var callback: AnyObject -> ()

    // RPC state
    var queue = dispatch_queue_create("com.levien.xi.CoreConnection", DISPATCH_QUEUE_SERIAL)
    var rpcIndex = 0
    var pending = Dictionary<Int, AnyObject? -> ()>()

    init(path: String, callback: AnyObject -> ()) {
        let task = NSTask()
        task.launchPath = path
        task.arguments = []
        let outPipe = NSPipe()
        task.standardOutput = outPipe
        sizeBuf = NSMutableData(length: 8)!
        let inPipe = NSPipe()
        task.standardInput = inPipe
        inHandle = inPipe.fileHandleForWriting
        recvBuf = NSMutableData(capacity: 65536)!
        self.callback = callback
        outPipe.fileHandleForReading.readabilityHandler = { handle in
            let data = handle.availableData
            self.recvHandler(data)
        }
        task.launch()
    }
    
    func recvHandler(data: NSData) {
        if data.length == 0 {
            print("eof")
            return
        }
        let scanStart = recvBuf.length
        recvBuf.appendData(data)
        let recvBufLen = recvBuf.length
        let recvBufBytes = UnsafeMutablePointer<UInt8>(recvBuf.mutableBytes)
        var i = 0
        for j in scanStart..<recvBufLen {
            // TODO: using memchr would probably be faster
            if recvBufBytes[j] == UInt8(ascii:"\n") {
                let dataPacket = recvBuf.subdataWithRange(NSRange(location: i, length: j + 1 - i))
                handleRaw(dataPacket)
                i = j + 1
            }
        }
        if i < recvBufLen {
            memmove(recvBufBytes, recvBufBytes + i, recvBufLen - i)
        }
        recvBuf.length = recvBufLen - i
    }

    func sendJson(json: AnyObject) {
        do {
            let data = try NSJSONSerialization.dataWithJSONObject(json, options: [])
            let mutdata = NSMutableData()
            mutdata.appendData(data)
            let nl = [0x0a as UInt8]
            mutdata.appendBytes(nl, length: 1)
            inHandle.writeData(mutdata)
        } catch _ {
            print("error serializing to json")
        }
    }

    func handleRaw(data: NSData) {
        do {
            let json = try NSJSONSerialization.JSONObjectWithData(data, options: .AllowFragments)
            //print("got \(json)")
            if !handleRpcResponse(json) {
                callback(json)
            }
        } catch _ {
            print("json error")
        }
    }

    func sendRpcAsync(method: String, params: AnyObject, callback: (AnyObject? -> ())? = nil) {
        var index = Int()
        var req = ["method": method, "params": params]
        if let callback = callback {
            dispatch_sync(queue) {
                req["id"] = self.rpcIndex
                index = self.rpcIndex
                self.rpcIndex += 1
                self.pending[index] = callback
            }
        }
        sendJson(req)
    }

    // send RPC synchronously, blocking until return
    func sendRpc(method: String, params: AnyObject) -> AnyObject? {
        let semaphore = dispatch_semaphore_create(0)
        var result: AnyObject? = nil
        sendRpcAsync(method, params: params) { r in
            result = r
            dispatch_semaphore_signal(semaphore)
        }
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER)
        return result
    }

    func handleRpcResponse(response: AnyObject) -> Bool {
        if let resp = response as? [String: AnyObject], let index = resp["id"] as? Int {
            var callback: (AnyObject? -> ())? = nil
            let result = resp["result"]
            dispatch_sync(queue) {
                callback = self.pending.removeValueForKey(index)
            }
            callback?(result)
            return true;
        } else {
            return false;
        }
    }
}
