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

/// The EditViewDataSource protocol describes the properties that an editView uses to determine how to render its contents.
protocol EditViewDataSource {
    var lines: LineCache { get }
    var styleMap: StyleMap { get }
    var textMetrics: TextDrawingMetrics { get }
}


class EditViewController: NSViewController, EditViewDataSource {

    @IBOutlet var editView: EditView!
    @IBOutlet var shadowView: ShadowView!
    @IBOutlet var scrollView: NSScrollView!
    @IBOutlet var documentView: NSClipView!
    
    var document: Document! {
        didSet {
            editView.document = document
        }
    }
    
    /// the height of the edit view, in lines.
    var linesHeight: Int {
        return Int(ceil((scrollView.contentView.bounds.size.height) / textMetrics.linespace))
    }

    var lines: LineCache = LineCache()
    var styleMap: StyleMap {
        return (NSApplication.shared().delegate as! AppDelegate).styleMap
    }

    //TODO: preferred font should be a user preference
    var textMetrics = TextDrawingMetrics(font: CTFontCreateWithName("InconsolataGo" as CFString?, 14, nil))
    
    // visible scroll region, exclusive of lastLine
    var firstLine: Int = 0
    var lastLine: Int = 0

    private var lastDragPosition: BufferPosition?
    /// handles autoscrolling when a drag gesture exists the window
    private var dragTimer: Timer?
    private var dragEvent: NSEvent?

    override func viewDidLoad() {
        super.viewDidLoad()
        editView.dataSource = self
        self.shadowView.mouseUpHandler = editView.mouseUp(with:)
        self.shadowView.mouseDraggedHandler = editView.mouseDragged(with:)
        scrollView.contentView.documentCursor = NSCursor.iBeam();
    }
    
    override func viewWillAppear() {
        super.viewWillAppear()
        updateEditViewScroll()
        NotificationCenter.default.addObserver(self, selector: #selector(EditViewController.boundsDidChangeNotification(_:)), name: NSNotification.Name.NSViewBoundsDidChange, object: scrollView.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(EditViewController.frameDidChangeNotification(_:)), name: NSNotification.Name.NSViewFrameDidChange, object: scrollView)
    }

    // this gets called when the user changes the font with the font book, for example
    override func changeFont(_ sender: Any?) {
        if let manager = sender as? NSFontManager {
            textMetrics = textMetrics.newMetricsForFontChange(fontManager: manager)
            self.editView.needsDisplay = true
            updateEditViewScroll()
        } else {
            Swift.print("changeFont: called with nil")
            return
        }
    }

    
    func boundsDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }
    
    func frameDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }

    fileprivate func visibleLineRange() -> (first: Int, last: Int) {
        let first = Int(floor(scrollView.contentView.bounds.origin.y / textMetrics.linespace))
        let height = Int(ceil((scrollView.contentView.bounds.size.height) / textMetrics.linespace))
        return (first, first+height)
    }

    // notifies core of new scroll position. Also effectively notifies core of current viewport size.
    fileprivate func updateEditViewScroll() {
        let (first, last) = visibleLineRange()
        if first != firstLine || last != lastLine {
            firstLine = first
            lastLine = last
            document.sendRpcAsync("scroll", params: [firstLine, lastLine])
        }
        shadowView?.updateScroll(scrollView.contentView.bounds, scrollView.documentView!.bounds)
    }
    
    fileprivate func requestLines(first: Int, last: Int) {
        let missing = lines.computeMissing(first, last)
        for (f, l) in missing {
            Swift.print("requesting missing: \(f)..\(l)")
            document.sendRpcAsync("request_lines", params: [f, l])
        }
    }
    
    // MARK: - Core Commands
    /// applies a set of line changes and redraws the view
    func update(_ updates: [String: AnyObject]) {
        lines.applyUpdate(update: updates)
        editView.heightConstraint?.constant = CGFloat(lines.height) * textMetrics.linespace + 2 * textMetrics.descent
        editView.showBlinkingCursor = editView.isFrontmostView
        editView.needsDisplay = true
    }

    func scrollTo(_ line: Int, _ col: Int) {
        editView.scrollTo(line, col)
    }
    
    // MARK: - System Events
    override func keyDown(with theEvent: NSEvent) {
        self.editView.inputContext?.handleEvent(theEvent);
    }
    
    override func mouseDown(with theEvent: NSEvent) {
        editView.removeMarkedText()
        editView.inputContext?.discardMarkedText()
        let position  = editView.bufferPositionFromPoint(theEvent.locationInWindow)
        lastDragPosition = position
        let flags = theEvent.modifierFlags.rawValue >> 16
        let clickCount = theEvent.clickCount
        document.sendRpcAsync("click", params: [position.line, position.column, flags, clickCount])
        dragTimer = Timer.scheduledTimer(timeInterval: TimeInterval(1.0/60), target: self, selector: #selector(_autoscrollTimerCallback), userInfo: nil, repeats: true)
        dragEvent = theEvent
    }
    
    override func mouseDragged(with theEvent: NSEvent) {
        editView.autoscroll(with: theEvent)
        let dragPosition = editView.bufferPositionFromPoint(theEvent.locationInWindow)
        if let last = lastDragPosition, last != dragPosition {
            lastDragPosition = dragPosition
            let flags = theEvent.modifierFlags.rawValue >> 16
            document?.sendRpcAsync("drag", params: [last.line, last.column, flags])
        }
        dragEvent = theEvent
    }
    
    override func mouseUp(with theEvent: NSEvent) {
        dragTimer?.invalidate()
        dragTimer = nil
        dragEvent = nil
    }
    
    func _autoscrollTimerCallback() {
        if let event = dragEvent {
            mouseDragged(with: event)
        }
    }
    
    // NSResponder (used mostly for paste)
    override func insertText(_ insertString: Any) {
        document.sendRpcAsync("insert", params: insertedStringToJson(insertString as! NSString))
    }

    // we intercept this method to check if we should open a new tab
    func newDocument(_ sender: NSMenuItem?) {
        // this tag is a property of the New Tab menu item, set in interface builder
        if sender?.tag == 10 {
            Document.preferredTabbingIdentifier = document.tabbingIdentifier
        } else {
            Document.preferredTabbingIdentifier = nil
        }
        // pass the message to the intended recipient
        NSDocumentController.shared().newDocument(sender)
    }

    // we override this to see if our view is empty, and should be reused for this open call
     func openDocument(_ sender: Any?) {
        if self.lines.isEmpty {
            Document._documentForNextOpenCall = self.document
        }
        Document.preferredTabbingIdentifier = nil
        NSDocumentController.shared().openDocument(sender)
    }
    
    // disable the New Tab menu item when running in 10.12
    override func validateMenuItem(_ menuItem: NSMenuItem) -> Bool {
        if menuItem.tag == 10 {
            if #available(OSX 10.12, *) { return true }
            return false
        }
        return true
    }
    
    // MARK: - Menu Items
    fileprivate func cutCopy(_ method: String) {
        let text = document.sendRpc(method, params: [])
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
        document.sendRpcAsync("undo", params: [])
    }
    
    func redo(_ sender: AnyObject?) {
        document.sendRpcAsync("redo", params: [])
    }

    // MARK: - Debug Methods
    @IBAction func debugRewrap(_ sender: AnyObject) {
        document.sendRpcAsync("debug_rewrap", params: [])
    }
    
    @IBAction func debugTestFGSpans(_ sender: AnyObject) {
        document.sendRpcAsync("debug_test_fg_spans", params: [])
    }
    
    @IBAction func debugRunPlugin(_ sender: AnyObject) {
        document.sendRpcAsync("debug_run_plugin", params: [])
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
