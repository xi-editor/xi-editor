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
    var callback: NSData -> ()

    init(path: String, callback: NSData -> ()) {
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
        outPipe.fileHandleForReading.readabilityHandler = { handle -> Void in
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
            for var j = 0; j < 8; j++ {
                size += (Int(recvBufBytes[i + j]) as Int) << (j * 8)
            }
            let dataPacket = recvBuf.subdataWithRange(NSRange(location: i + 8, length: size))
            callback(dataPacket)
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
        for var i = 0; i < 8; i++ {
            sizeBytes[i] = UInt8((length >> (i * 8)) & 0xff)
        }
        inHandle.writeData(sizeBuf)
        inHandle.writeData(data)
    }

}
