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

class Dispatcher {
    let coreConnection: CoreConnection

    init(coreConnection: CoreConnection) {
        self.coreConnection = coreConnection
    }

    func dispatchSync<E: Event, O>(_ event: E) -> O {
        let rpc = event.rpcRepresentation
        return coreConnection.sendRpc(rpc.method, params: rpc.params) as! O
    }

    func dispatchAsync<E: Event, O>(_ event: E) -> O {
        let rpc = event.rpcRepresentation
        return coreConnection.sendRpcAsync(rpc.method, params: rpc.params) as! O
    }
}

typealias RpcRepresentation = (method: String, params: AnyObject)

enum EventDispatchMethod {
    case sync
    case async
}

protocol Event {
    associatedtype Output

    var method: String { get }
    var params: AnyObject? { get }
    var rpcRepresentation: RpcRepresentation { get }
    var dispatchMethod: EventDispatchMethod { get }

    func dispatch(_ dispatcher: Dispatcher) -> Output
}

extension Event {
    var rpcRepresentation: RpcRepresentation {
        return (method, params ?? [] as AnyObject)
    }

    func dispatch(_ dispatcher: Dispatcher) -> Output {
        switch dispatchMethod {
        case .sync: return dispatcher.dispatchSync(self)
        case .async: return dispatcher.dispatchAsync(self)
        }
    }
}

typealias TabIdentifier = String

enum Events { // namespace
    struct NewTab: Event {
        typealias Output = String

        let method = "new_tab"
        let params: AnyObject? = nil
        let dispatchMethod = EventDispatchMethod.sync
    }

    struct DeleteTab: Event {
        typealias Output = Void

        let tabId: TabIdentifier

        let method = "delete_tab"
        var params: AnyObject? { return ["tab": tabId] as AnyObject }
        let dispatchMethod = EventDispatchMethod.async
    }
}
