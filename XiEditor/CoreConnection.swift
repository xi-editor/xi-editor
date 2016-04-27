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
        recvBuf.appendData(data)
        let recvBufLen = recvBuf.length
        let recvBufBytes = UnsafeMutablePointer<UInt8>(recvBuf.mutableBytes)
        var i = 0
        while true {
            if recvBufLen < i + 8 {
                break
            }
            var size = 0
            for j in 0 ..< 8 {
                size += (Int(recvBufBytes[i + j]) as Int) << (j * 8)
            }
            if recvBufLen < i + 8 + size {
                break
            }
            let dataPacket = recvBuf.subdataWithRange(NSRange(location: i + 8, length: size))
            handleRaw(dataPacket)
            i += 8 + size
        }
        if i < recvBufLen {
            memmove(recvBufBytes, recvBufBytes + i, recvBufLen - i)
        }
        recvBuf.length = recvBufLen - i
    }

    func send(data: NSData) {
        let length = data.length
        let sizeBytes = UnsafeMutablePointer<UInt8>(sizeBuf.mutableBytes)
        for i in 0 ..< 8 {
            sizeBytes[i] = UInt8((length >> (i * 8)) & 0xff)
        }
        inHandle.writeData(sizeBuf)
        inHandle.writeData(data)
    }

    func sendJson(json: AnyObject) {
        do {
            let data = try NSJSONSerialization.dataWithJSONObject(json, options: [])
            send(data)
        } catch _ {
            print("error serializing to json")
        }
    }

    func handleRaw(data: NSData) {
        do {
            let json = try NSJSONSerialization.JSONObjectWithData(data, options: .AllowFragments)
            //print("got \(json)")
            if let response = json as? [AnyObject] where response.count == 2, let cmd = response[0] as? NSString {
                if cmd == "rpc_response" {
                    handleRpcResponse(response[1])
                    return
                }
            }
            callback(json)
        } catch _ {
            print("json error")
        }
    }

    func sendRpcAsync(request: AnyObject, callback: AnyObject? -> ()) {
        var index = Int()
        dispatch_sync(queue) {
            index = self.rpcIndex
            self.rpcIndex += 1
            self.pending[index] = callback
        }
        sendJson(["rpc", ["index": index, "request": request]])
    }

    // send RPC synchronously, blocking until return
    func sendRpc(request: AnyObject) -> AnyObject? {
        let semaphore = dispatch_semaphore_create(0)
        var result: AnyObject? = nil
        sendRpcAsync(request) { r in
            result = r
            dispatch_semaphore_signal(semaphore)
        }
        dispatch_semaphore_wait(semaphore, DISPATCH_TIME_FOREVER)
        return result
    }

    func handleRpcResponse(response: AnyObject) {
        if let resp = response as? [String: AnyObject], let index = resp["index"] as? Int {
            var callback: (AnyObject? -> ())? = nil
            let result = resp["result"]
            dispatch_sync(queue) {
                callback = self.pending.removeValueForKey(index)
            }
            callback?(result)
        }
    }
}
