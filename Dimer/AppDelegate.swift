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

func eventToJson(event: NSEvent) -> AnyObject {
    return ["key", ["keycode": Int(event.keyCode), "chars": event.characters!]]
}

@NSApplicationMain
class AppDelegate: NSObject, NSApplicationDelegate {

    var coreConnection: CoreConnection?
    var appWindowController: AppWindowController?

    
    func applicationDidFinishLaunching(aNotification: NSNotification) {
        // show main app window
        appWindowController = AppWindowController.init(windowNibName: "AppWindowController")
        appWindowController?.showWindow(self)

        let corePath = NSBundle.mainBundle().pathForResource("python/dimercore", ofType: "py")
        if let corePath = corePath {
            coreConnection = CoreConnection(path: corePath) { [weak self] data -> () in
                self?.handleCoreCmd(data)
            }
        }
        appWindowController?.eventCallback = { [weak self] event -> () in
            self?.sendJson(eventToJson(event))
        }
    }
    
    func sendJson(json: AnyObject) {
        do {
            let data = try NSJSONSerialization.dataWithJSONObject(json, options: [])
            self.coreConnection?.send(data)
        } catch _ {
        }
    }
    
    func handleCoreCmd(data: NSData) {
        do {
            let json = try NSJSONSerialization.JSONObjectWithData(data, options: .AllowFragments)
            print("got \(json)")
            if let response = json as? [AnyObject] where response.count == 2, let cmd = response[0] as? NSString {
                if cmd == "settext" {
                    appWindowController?.editView.mySetText(response[1] as! String)
                }
            }
        } catch _ {
            print("json error")
        }
    }

    func applicationWillTerminate(aNotification: NSNotification) {
        // Insert code here to tear down your application
    }


}

