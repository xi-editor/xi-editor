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

class EditViewController: NSViewController {

    @IBOutlet var editView: EditView!
    @IBOutlet var shadowView: ShadowView!
    @IBOutlet var scrollView: NSScrollView!
    @IBOutlet var documentView: NSClipView!
    
    var document: Document! {
        didSet {
            editView.document = document
        }
    }

    // visible scroll region, exclusive of lastLine
    var firstLine: Int = 0
    var lastLine: Int = 0

    override func viewDidLoad() {
        super.viewDidLoad()
        self.shadowView.mouseUpHandler = editView.mouseUp(with:)
        self.shadowView.mouseDraggedHandler = editView.mouseDragged(with:)
        scrollView.contentView.documentCursor = NSCursor.iBeam();
        
        NotificationCenter.default.addObserver(self, selector: #selector(EditViewController.boundsDidChangeNotification(_:)), name: NSNotification.Name.NSViewBoundsDidChange, object: scrollView.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(EditViewController.frameDidChangeNotification(_:)), name: NSNotification.Name.NSViewFrameDidChange, object: scrollView)
    }
    
    func boundsDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }
    
    func frameDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }

    
    fileprivate func updateEditViewScroll() {
        let first = Int(floor(scrollView.contentView.bounds.origin.y / editView.linespace))
        let height = Int(ceil((scrollView.contentView.bounds.size.height) / editView.linespace))
        let last = first + height
        if first != firstLine || last != lastLine {
            firstLine = first
            lastLine = last
            document?.sendRpcAsync("scroll", params: [firstLine, lastLine])
        }
        shadowView?.updateScroll(scrollView.contentView.bounds, scrollView.documentView!.bounds)
    }
    
    // MARK: - Core Commands
    func update(_ content: [String: AnyObject]) {
        editView.updateSafe(update: content)
    }

    func scrollTo(_ line: Int, _ col: Int) {
        editView.scrollTo(line, col)
    }
    
    // MARK: - System Events
    override func keyDown(with theEvent: NSEvent) {
        self.editView.inputContext?.handleEvent(theEvent);
    }
    
    // NSResponder (used mostly for paste)
    override func insertText(_ insertString: Any) {
        document?.sendRpcAsync("insert", params: insertedStringToJson(insertString as! NSString))
    }
    // MARK: - Menu Items
    
    fileprivate func cutCopy(_ method: String) {
        let text = document?.sendRpc(method, params: [])
        if let text = text as? String {
            let pasteboard = NSPasteboard.general()
            pasteboard.clearContents()
            pasteboard.writeObjects([text as NSPasteboardWriting])
        }
    }
    
    func cut(_ sender: AnyObject?) {
        cutCopy("cut")
    }
    
    func copy(_ sender: AnyObject?) {
        cutCopy("copy")
    }
    
    func paste(_ sender: AnyObject?) {
        let pasteboard = NSPasteboard.general()
        if let items = pasteboard.pasteboardItems {
            for element in items {
                if let str = element.string(forType: "public.utf8-plain-text") {
                    insertText(str)
                    break
                }
            }
        }
    }
    
    func undo(_ sender: AnyObject?) {
        document?.sendRpcAsync("undo", params: [])
    }
    
    func redo(_ sender: AnyObject?) {
        document?.sendRpcAsync("redo", params: [])
    }

    // MARK: - Debug Methods
    @IBAction func debugRewrap(_ sender: AnyObject) {
        document?.sendRpcAsync("debug_rewrap", params: [])
    }
    
    @IBAction func debugTestFGSpans(_ sender: AnyObject) {
        document?.sendRpcAsync("debug_test_fg_spans", params: [])
    }
    
    @IBAction func debugRunPlugin(_ sender: AnyObject) {
        document?.sendRpcAsync("debug_run_plugin", params: [])
    }
}

// we set this in Document.swift when we load a new window or tab.
//TODO: will have to think about whether this will work with splits
extension EditViewController: NSWindowDelegate {
    func windowDidBecomeKey(_ notification: Notification) {
        editView.isFrontmostView = true
    }

    func windowDidResignKey(_ notification: Notification) {
        editView.isFrontmostView = false
    }
}
