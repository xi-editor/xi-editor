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

class AppWindowController: NSWindowController {

    convenience init() {
        self.init(windowNibName: "AppWindowController")
    }

    @IBOutlet weak var editView: EditView!
    @IBOutlet weak var scrollView: NSScrollView!
    @IBOutlet weak var shadowView: ShadowView!

    // TODO: do we need to wire this explicitly, or is it ok to get delegate from shared NSApplication?
    weak var appDelegate: AppDelegate!

    var dispatcher: Dispatcher!
    
    var filename: String? {
        didSet {
            if let filename = filename {
                let url = URL(fileURLWithPath: filename)
                let lastComponent = url.lastPathComponent;
                window?.title = lastComponent
            }
        }
    }

    func visualConstraint(_ views: [String : NSView], _ format: String) {
        let constraints = NSLayoutConstraint.constraints(withVisualFormat: format, options: .alignAllTop, metrics: nil, views: views)
        NSLayoutConstraint.activate(constraints)
    }

    override func windowDidLoad() {
        super.windowDidLoad()
        //window?.backgroundColor = NSColor.whiteColor()

        let tabName = Events.NewTab().dispatch(dispatcher)
        editView.coreConnection = dispatcher.coreConnection
        editView.tabName = tabName
        appDelegate.registerTab(tabName, controller: self)
        
        scrollView.contentView.documentCursor = NSCursor.iBeam();

        // set up autolayout constraints
        let views = ["editView": editView, "clipView": scrollView.contentView] as [String : Any]
        visualConstraint(views as! [String : NSView], "H:[editView(>=clipView)]")
        visualConstraint(views as! [String : NSView], "V:[editView(>=clipView)]")

        NotificationCenter.default.addObserver(self, selector: #selector(AppWindowController.boundsDidChangeNotification(_:)), name: NSNotification.Name.NSViewBoundsDidChange, object: scrollView.contentView)
        NotificationCenter.default.addObserver(self, selector: #selector(AppWindowController.frameDidChangeNotification(_:)), name: NSNotification.Name.NSViewFrameDidChange, object: scrollView)
        updateEditViewScroll()
    }

    func windowWillClose(_: Notification) {
        guard let tabName = editView.tabName
            else { return }

        Events.DeleteTab(tabId: tabName).dispatch(dispatcher)
        appDelegate.unregisterTab(tabName)
    }

    func boundsDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }

    func frameDidChangeNotification(_ notification: Notification) {
        updateEditViewScroll()
    }

    func updateEditViewScroll() {
        editView?.updateScroll(scrollView.contentView.bounds)
        shadowView?.updateScroll(scrollView.contentView.bounds, scrollView.documentView!.bounds)
    }

    func saveDocument(_ sender: AnyObject) {
        guard filename != nil else {
            saveDocumentAs(sender)
            return
        }

        editView.sendRpcAsync("save", params: ["filename": filename!] as AnyObject)
    }
    
    func saveDocumentAs(_ sender: AnyObject) {
        let fileDialog = NSSavePanel()
        if fileDialog.runModal() == NSFileHandlingPanelOKButton {
            if let path = fileDialog.url?.path {
                filename = path
                saveDocument(sender)
            }
        }
    }

    // the ShadowView sometimes steals drag events, so forward them back to the edit view
    func handleMouseDragged(_ theEvent: NSEvent) {
        editView.mouseDragged(with: theEvent)
    }

    func handleMouseUp(_ theEvent: NSEvent) {
        editView.mouseUp(with: theEvent)
    }
}

// AppWindowController.xib makes us the window's delegate (as nib owner), as well as its controler.
extension AppWindowController: NSWindowDelegate {
    func windowDidBecomeKey(_ notification: Notification) {
        editView.updateIsFrontmost(true)
    }
    func windowDidResignKey(_ notification: Notification) {
        editView.updateIsFrontmost(false);
        
    }
}
