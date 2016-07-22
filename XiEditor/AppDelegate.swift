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

    var appWindowController: AppWindowController?
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

        appWindowController = AppWindowController()
        appWindowController?.dispatcher = dispatcher
        appWindowController?.showWindow(self)
    }

    func handleCoreCmd(json: AnyObject) {
        guard let obj = json as? [String : AnyObject],
            method = obj["method"] as? String,
            params = obj["params"]
            else { print("unknown json from core:", json); return }

        handleRpc(method, params: params)
    }

    func handleRpc(method: String, params: AnyObject) {
        if method == "update" {
            if let obj = params as? [String : AnyObject], let update = obj["update"] as? [String : AnyObject] {
                // TODO: dispatch to appropriate editView based on obj["tab"]
                self.appWindowController?.editView.updateSafe(update)
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
        appWindowController?.editView.sendRpcAsync("open", params: ["filename": filename])
        return true  // TODO: should be RPC instead of async, plumb errors
    }

    func applicationWillTerminate(aNotification: NSNotification) {
        // Insert code here to tear down your application
    }

}
