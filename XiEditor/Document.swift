//
//  Document.swift
//  XiEditor
//
//  Created by Brandon Titus on 11/2/16.
//  Copyright © 2016 Google. All rights reserved.
//

import Cocoa

//enum Action {
//    case DeleteBackward
//    case Insert(AnyObject)
//    case Key(Int, String, UInt)
//    case Cut
//    case Copy
//    case Undo
//    case Redo
//    case Click(Int, Int, UInt, Int)
//    case Drag(Int, Int, UInt)
//    case Scroll(Int, Int)
//    case RenderLines(Int, Int)
//    case Unknown(String)
//    
//    // Debug
//    case DebugRewrap
//    case DebugTestFGSpans
//    case DebugRunPlugin
//    
//    var method: String {
//        switch self {
//        case .DeleteBackward:
//            return "delete_backward"
//        case .Insert(_):
//            return "insert"
//        case .Key(_, _, _):
//            return "key"
//        case .Cut:
//            return "cut"
//        case .Copy:
//            return "copy"
//        case .Undo:
//            return "undo"
//        case .Redo:
//            return "redo"
//        case .Click(_, _, _, _):
//            return "click"
//        case .Drag(_, _, _):
//            return "drag"
//        case .Scroll(_, _):
//            return "scroll"
//        case .RenderLines(_, _):
//            return "render_lines"
//        case .Unknown(let command):
//            return command
//        case .DebugRewrap:
//            return "debug_rewrap"
//        case .DebugTestFGSpans:
//            return "debug_test_fg_spans"
//        case .DebugRunPlugin:
//            return "debug_run_plugin"
//        }
//    }
//    
//    //NOTE: Is there any way we can make this [String: AnyObject]?
//    var params: AnyObject {
//        switch self {
//        case .DeleteBackward:
//            return [:]
//        case .Insert(let string):
//            return ["chars": string]
//        case .Key(let keyCode, let characters, let flags):
//            return ["keycode": keyCode,
//             "chars": characters,
//             "flags": flags]
//        case .Cut:
//            return [:]
//        case .Copy:
//            return [:]
//        case .Undo:
//            return [:]
//        case .Redo:
//            return [:]
//        case .Click(let line, let col, let flags, let clickCount):
//            return [line, col, flags, clickCount]
//        case .Drag(let line, let col, let flags):
//            return [line, col, flags]
//        case .Scroll(let firstLine, let lastLine):
//            return [firstLine, lastLine]
//        case .RenderLines(let firstLine, let lastLine):
//            return ["first_line": firstLine, "last_line": lastLine]
//        case .Unknown(_):
//            return [:]
//        case .DebugRewrap:
//            return [:]
//        case .DebugTestFGSpans:
//            return [:]
//        case .DebugRunPlugin:
//            return [:]
//        }
//    }
//}

class Document: NSDocument {

    /*
    override var windowNibName: String? {
        // Override returning the nib file name of the document
        // If you need to use a subclass of NSWindowController or if your document supports multiple NSWindowControllers, you should remove this method and override -makeWindowControllers instead.
        return "Document"
    }
    */
    
    var dispatcher: Dispatcher?
    var tabName: String?
    
    var filename: String?
    
    override init() {
        super.init()
        
        dispatcher = (NSApplication.sharedApplication().delegate as? AppDelegate)?.dispatcher
    }
    
    override func makeWindowControllers() {
        // Returns the Storyboard that contains your Document window.
        let storyboard = NSStoryboard(name: "Main", bundle: nil)
        let windowController = storyboard.instantiateControllerWithIdentifier("Document Window Controller") as! NSWindowController
        tabName = Events.NewTab().dispatch(dispatcher!)
        let editViewController = windowController.contentViewController as? EditViewController
        editViewController?.editView.document = self
        windowController.window?.delegate = editViewController

        if let filename = filename {
            open(filename)
        }

        self.addWindowController(windowController)
    }
    
    override func readFromURL(url: NSURL, ofType typeName: String) throws {
        filename = url.path
    }
    
    override func saveToURL(url: NSURL, ofType typeName: String, forSaveOperation saveOperation: NSSaveOperationType, completionHandler: (NSError?) -> Void) {
        save(url.path!)
        
        // An RPC Call received to indicate Save can be used to call this completion
        completionHandler(nil)
    }
    
    override func close() {
        super.close()
        
        guard let tabName = tabName
            else { return }

        Events.DeleteTab(tabId: tabName).dispatch(dispatcher!)
    }
    
    override var entireFileLoaded: Bool {
        return false
    }
    
    override class func autosavesInPlace() -> Bool {
        return false
    }

    private func open(filename: String) {
        sendRpcAsync("open", params: ["filename": filename])
    }
    
    private func save(filename: String) {
        sendRpcAsync("save", params: ["filename": filename])
    }
        
//    func sendRPCAction(action: Action) -> AnyObject? {
//        return sendRpc(action.method, params: action.params)
//    }
//    
//    func sendRPCActionAsync(action: Action) {
//        sendRpcAsync(action.method, params: action.params)
//    }
    
    func sendRpcAsync(method: String, params: AnyObject) {
        let inner = ["method": method, "params": params, "tab": tabName ?? ""] as [String : AnyObject]
        dispatcher?.coreConnection.sendRpcAsync("edit", params: inner)
    }
    
    func sendRpc(method: String, params: AnyObject) -> AnyObject? {
        let inner = ["method": method, "params": params, "tab": tabName ?? ""] as [String : AnyObject]
        return dispatcher?.coreConnection.sendRpc("edit", params: inner)
    }
    
    func update(content: [String: AnyObject]) {
        for windowController in windowControllers {
            (windowController.contentViewController as? EditViewController)?.editView.updateSafe(content)
        }
    }

}
