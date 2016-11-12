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

import Cocoa

@NSApplicationMain
class AppDelegate: NSObject, NSApplicationDelegate {

    var dispatcher: Dispatcher?

    func applicationWillFinishLaunching(aNotification: NSNotification) {

        guard let corePath = NSBundle.mainBundle().pathForResource("xi-core", ofType: "")
            else { fatalError("XI Core not found") }

        let dispatcher: Dispatcher = {
            let coreConnection = CoreConnection(path: corePath) { [weak self] (json: AnyObject) -> Void in
                self?.handleCoreCmd(json)
            }

            return Dispatcher(coreConnection: coreConnection)
        }()

        self.dispatcher = dispatcher
    }
    
    func handleCoreCmd(json: AnyObject) {
        guard let obj = json as? [String : AnyObject],
            method = obj["method"] as? String,
            params = obj["params"]
            else { print("unknown json from core:", json); return }

        handleRpc(method, params: params)
    }

    func handleRpc(method: String, params: AnyObject) {
        switch method {
        case "update":
            if let obj = params as? [String : AnyObject], let update = obj["update"] as? [String : AnyObject] {
                guard let tab = obj["tab"] as? String
                    else { print("tab missing from update event"); return }
                
                for document in NSApplication.sharedApplication().orderedDocuments {
                    let doc = document as? Document
                    if doc?.tabName == tab {
                        doc?.update(update)
                    }
                }
            }
        case "alert":
            if let obj = params as? [String : AnyObject], let msg = obj["msg"] as? String {
                dispatch_async(dispatch_get_main_queue(), {
                    let alert =  NSAlert.init()
                    #if swift(>=2.3)
                        alert.alertStyle = .Informational
                    #else
                        alert.alertStyle = .InformationalAlertStyle
                    #endif
                    alert.messageText = msg
                    alert.runModal()
                });
            }
        default:
            print("unknown method from core:", method)
        }
    }

    func applicationWillTerminate(aNotification: NSNotification) {
        // Insert code here to tear down your application
    }

}
