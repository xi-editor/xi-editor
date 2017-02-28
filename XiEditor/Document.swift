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


class Document: NSDocument {

    var dispatcher: Dispatcher!
    var tabName: String
    
    /// an initial backend update to be rendered on load
    var pendingUpdate: [String: AnyObject]? = nil
    var editViewController: EditViewController? {
        didSet {
            if let new = editViewController, let content = pendingUpdate {
                new.update(content)
                pendingUpdate = nil
            }
        }
    }

    override init() {
        dispatcher = (NSApplication.shared().delegate as? AppDelegate)?.dispatcher
        tabName = Events.NewTab().dispatch(dispatcher!)
        super.init()
        // I'm not 100% sure this is necessary but it can't _hurt_
        self.hasUndoManager = false
    }
    
    override func makeWindowControllers() {
        // Returns the Storyboard that contains your Document window.
        let storyboard = NSStoryboard(name: "Main", bundle: nil)
        let windowController = storyboard.instantiateController(withIdentifier: "Document Window Controller") as! NSWindowController
        self.editViewController = windowController.contentViewController as? EditViewController
        editViewController?.editView.document = self

        windowController.window?.delegate = editViewController
        //FIXME: some saner way of positioning new windows. maybe based on current window size, with some checks to not completely obscure an existing window?
        // also awareness of multiple screens (prefer to open on currently active screen)
        let screenHeight = windowController.window?.screen?.frame.height ?? 800
        let windowHeight: CGFloat = 800
        windowController.window?.setFrame(NSRect(x: 200, y: screenHeight - windowHeight - 200, width: 700, height: 800), display: true)

        self.addWindowController(windowController)
    }
    
    override func read(from url: URL, ofType typeName: String) throws {
        self.open(url.path)
    }
    
    override func save(to url: URL, ofType typeName: String, for saveOperation: NSSaveOperationType, completionHandler: @escaping (Error?) -> Void) {
        self.save(url.path)
        //TODO: save operations should report success, and we should pass any errors to the completion handler
        completionHandler(nil)
    }
    
    override func close() {
        super.close()
        Events.DeleteTab(tabId: tabName).dispatch(dispatcher!)
    }
    
    override var isEntireFileLoaded: Bool {
        return false
    }
    
    override class func autosavesInPlace() -> Bool {
        return false
    }

    fileprivate func open(_ filename: String) {
        sendRpcAsync("open", params: ["filename": filename])
    }
    
    fileprivate func save(_ filename: String) {
        sendRpcAsync("save", params: ["filename": filename])
    }
    
    
    func sendRpcAsync(_ method: String, params: Any) {
        let inner = ["method": method as AnyObject, "params": params, "tab": tabName as AnyObject] as [String : Any]
        dispatcher?.coreConnection.sendRpcAsync("edit", params: inner)
    }
    
    func sendRpc(_ method: String, params: Any) -> Any? {
        let inner = ["method": method as AnyObject, "params": params, "tab": tabName as AnyObject] as [String : Any]
        return dispatcher?.coreConnection.sendRpc("edit", params: inner)
    }

    func update(_ content: [String: AnyObject]) {
        if let editVC = editViewController {
            editVC.update(content)
        } else {
            pendingUpdate = content
        }
    }
}
