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

    var coreConnection: CoreConnection?
    var appWindowController: AppWindowController?
    
    func applicationWillFinishLaunching(aNotification: NSNotification) {
        // show main app window
        appWindowController = AppWindowController(windowNibName: "AppWindowController")

        let corePath = NSBundle.mainBundle().pathForResource("xicore", ofType: "")
        if let corePath = corePath {
            coreConnection = CoreConnection(path: corePath) { [weak self] data -> () in
                self?.handleCoreCmd(data)
            }
        }
        appWindowController?.coreConnection = coreConnection

        appWindowController?.showWindow(self)
    }
    
    func handleCoreCmd(json: AnyObject) {
        if let response = json as? [AnyObject] where response.count == 2, let cmd = response[0] as? NSString {
            if cmd == "settext" {
                self.appWindowController?.editView.updateSafe(response[1] as! [String: AnyObject])
            }
        }
    }

    func openDocument(sender: AnyObject) {
        let fileDialog = NSOpenPanel()
        if fileDialog.runModal() == NSFileHandlingPanelOKButton {
            if let path = fileDialog.URL?.path {
                application(NSApp, openFile: path)
                NSDocumentController.sharedDocumentController().noteNewRecentDocumentURL(fileDialog.URL!);
            }
        }
    }

    func application(sender: NSApplication, openFile filename: String) -> Bool {
        appWindowController?.filename = filename
        coreConnection?.sendJson(["open", filename])
        return true  // TODO: should be RPC instead of async, plumb errors
    }

    func applicationWillTerminate(aNotification: NSNotification) {
        // Insert code here to tear down your application
    }

}

