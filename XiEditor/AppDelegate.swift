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
    var styleMap: StyleMap = StyleMap()

    func applicationWillFinishLaunching(_ aNotification: Notification) {

        guard let corePath = Bundle.main.path(forResource: "xi-core", ofType: "")
            else { fatalError("XI Core not found") }

        let dispatcher: Dispatcher = {
            let coreConnection = CoreConnection(path: corePath) { [weak self] (json: Any) -> Void in
                self?.handleCoreCmd(json)
            }

            return Dispatcher(coreConnection: coreConnection)
        }()

        self.dispatcher = dispatcher
    }
    
    func handleCoreCmd(_ json: Any) {
        guard let obj = json as? [String : Any],
            let method = obj["method"] as? String,
            let params = obj["params"]
            else { print("unknown json from core:", json); return }

        handleRpc(method, params: params)
    }

    func handleRpc(_ method: String, params: Any) {
        switch method {
        case "update":
            if let obj = params as? [String : AnyObject], let update = obj["update"] as? [String : AnyObject] {
                guard let tab = obj["tab"] as? String
                    else { print("tab missing from update event"); return }
                
                for document in NSApplication.shared().orderedDocuments {
                    let doc = document as? Document
                    if doc?.tabName == tab {
                        doc?.update(update)
                    }
                }
            }
//FIXME: convert to new document model
        case "scroll_to":
            if let obj = params as? [String : AnyObject], let line = obj["line"] as? Int, let col = obj["col"] as? Int {
                guard let tab = obj["tab"] as? String
                    else { print("tab missing from update event"); return }
                
                for document in NSApplication.shared().orderedDocuments {
                    let doc = document as? Document
                    if doc?.tabName == tab {
                        doc?.editView?.scrollTo(line, col)
                        break
                    }
                }
//                guard let document = appWindowControllers[tab]
//                    else { print("tab " + tab + " not registered"); return }
//                appWindowController.editView.scrollTo(line, col)
            }
        case "def_style":
            if let obj = params as? [String : AnyObject] {
                styleMap.defStyle(json: obj)
            }
        case "alert":
            if let obj = params as? [String : AnyObject], let msg = obj["msg"] as? String {
                DispatchQueue.main.async(execute: {
                    let alert =  NSAlert.init()
                    #if swift(>=2.3)
                        alert.alertStyle = .informational
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

    func applicationWillTerminate(_ aNotification: Notification) {
        // Insert code here to tear down your application
    }

}
