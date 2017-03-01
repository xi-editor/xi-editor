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

struct PendingNotification {
    let method: String
    let params: Any
}

class Document: NSDocument {

    var dispatcher: Dispatcher!
    var tabName: String? {
        didSet {
            guard tabName != nil else { return }
            // apply initial updates when tabName is set
            for pending in self.pendingNotifications {
                self.sendRpcAsync(pending.method, params: pending.params)
            }
            self.pendingNotifications.removeAll()
        }
    }
    
	var pendingNotifications: [PendingNotification] = [];
    var editViewController: EditViewController?

    /// used to keep track of whether we're in the process of reusing an empty window
    fileprivate var _skipShowingWindow = false

    override init() {
        dispatcher = (NSApplication.shared().delegate as? AppDelegate)?.dispatcher
        super.init()
        // I'm not 100% sure this is necessary but it can't _hurt_
        self.hasUndoManager = false
    }
 
    override func makeWindowControllers() {
        var windowController: NSWindowController!
        // check to see if we should reuse another document's window
        if let delegate = (NSApplication.shared().delegate as? AppDelegate), let existing = delegate._documentForNextOpenCall {
            assert(existing.windowControllers.count == 1, "each document should only and always own a single windowController")
            windowController = existing.windowControllers[0]
            delegate._documentForNextOpenCall = nil
            // if we're reusing an existing window, we want to noop on the `showWindows()` call we receive from the DocumentController
            _skipShowingWindow = true
        } else {
            // if we aren't reusing, create a new window as normal:
            let storyboard = NSStoryboard(name: "Main", bundle: nil)
            windowController = storyboard.instantiateController(withIdentifier: "Document Window Controller") as! NSWindowController
            
            //FIXME: some saner way of positioning new windows. maybe based on current window size, with some checks to not completely obscure an existing window?
            // also awareness of multiple screens (prefer to open on currently active screen)
            let screenHeight = windowController.window?.screen?.frame.height ?? 800
            let windowHeight: CGFloat = 800
            windowController.window?.setFrame(NSRect(x: 200, y: screenHeight - windowHeight - 200, width: 700, height: 800), display: true)
        }

        self.editViewController = windowController.contentViewController as? EditViewController
        editViewController?.document = self
        windowController.window?.delegate = editViewController
        self.addWindowController(windowController)

        Events.NewTab().dispatchWithCallback(dispatcher!) { (tabName) in
            DispatchQueue.main.async {
            self.tabName = tabName
            }
        }
    }

    override func showWindows() {
        // part of our code to reuse existing windows when opening documents
        assert(windowControllers.count == 1, "documents should have a single window controller")
        if !(_skipShowingWindow) {
            super.showWindows()
        } else {
            _skipShowingWindow = false
        }
    }
    
    override func read(from url: URL, ofType typeName: String) throws {
        self.open(url.path)
    }
    
    override func save(to url: URL, ofType typeName: String, for saveOperation: NSSaveOperationType, completionHandler: @escaping (Error?) -> Void) {
        self.fileURL = url
        self.save(url.path)
        //TODO: save operations should report success, and we should pass any errors to the completion handler
        completionHandler(nil)
    }
    
    override func close() {
        super.close()
        Events.DeleteTab(tabId: tabName!).dispatch(dispatcher!)
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
    
    /// Send a notification specific to the tab. If the tab name hasn't been set, then the
    /// notification is queued, and sent when the tab name arrives.
    func sendRpcAsync(_ method: String, params: Any) {
        if let tabName = tabName {
            let inner = ["method": method, "params": params, "tab": tabName] as [String : Any]
            dispatcher?.coreConnection.sendRpcAsync("edit", params: inner)
        } else {
            pendingNotifications.append(PendingNotification(method: method, params: params))
        }
    }

    /// Note: this is a blocking call, and will also fail if the tab name hasn't been set yet.
    /// We should try to migrate users to either fully async or callback based approaches.
    func sendRpc(_ method: String, params: Any) -> Any? {
        let inner = ["method": method as AnyObject, "params": params, "tab": tabName as AnyObject] as [String : Any]
        return dispatcher?.coreConnection.sendRpc("edit", params: inner)
    }

    func update(_ content: [String: AnyObject]) {
        if let editVC = editViewController {
            editVC.update(content)
        }
    }
}
