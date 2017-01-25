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
                let url = NSURL(fileURLWithPath: filename)
                if let lastComponent = url.lastPathComponent {
                    window?.title = lastComponent
                }
            }
        }
    }

    func visualConstraint(views: [String : NSView], _ format: String) {
        let constraints = NSLayoutConstraint.constraintsWithVisualFormat(format, options: .AlignAllTop, metrics: nil, views: views)
        NSLayoutConstraint.activateConstraints(constraints)
    }

    override func windowDidLoad() {
        super.windowDidLoad()
        //window?.backgroundColor = NSColor.whiteColor()

        let tabName = Events.NewTab().dispatch(dispatcher)
        editView.coreConnection = dispatcher.coreConnection
        editView.tabName = tabName
        appDelegate.registerTab(tabName, controller: self)
        
        scrollView.contentView.documentCursor = NSCursor.IBeamCursor();

        // set up autolayout constraints
        let views = ["editView": editView, "clipView": scrollView.contentView]
        visualConstraint(views, "H:[editView(>=clipView)]")
        visualConstraint(views, "V:[editView(>=clipView)]")

        NSNotificationCenter.defaultCenter().addObserver(self, selector: #selector(AppWindowController.boundsDidChangeNotification(_:)), name: NSViewBoundsDidChangeNotification, object: scrollView.contentView)
        NSNotificationCenter.defaultCenter().addObserver(self, selector: #selector(AppWindowController.frameDidChangeNotification(_:)), name: NSViewFrameDidChangeNotification, object: scrollView)
        updateEditViewScroll()
    }

    func windowWillClose(_: NSNotification) {
        guard let tabName = editView.tabName
            else { return }

        Events.DeleteTab(tabId: tabName).dispatch(dispatcher)
        appDelegate.unregisterTab(tabName)
    }

    func boundsDidChangeNotification(notification: NSNotification) {
        updateEditViewScroll()
    }

    func frameDidChangeNotification(notification: NSNotification) {
        updateEditViewScroll()
    }

    func updateEditViewScroll() {
        editView?.updateScroll(scrollView.contentView.bounds)
        shadowView?.updateScroll(scrollView.contentView.bounds, scrollView.documentView!.bounds)
    }

    func saveDocument(sender: AnyObject) {
        guard filename != nil else {
            saveDocumentAs(sender)
            return
        }

        editView.sendRpcAsync("save", params: ["filename": filename!])
    }
    
    func saveDocumentAs(sender: AnyObject) {
        let fileDialog = NSSavePanel()
        if fileDialog.runModal() == NSFileHandlingPanelOKButton {
            if let path = fileDialog.URL?.path {
                filename = path
                saveDocument(sender)
            }
        }
    }

    // the ShadowView sometimes steals drag events, so forward them back to the edit view
    func handleMouseDragged(theEvent: NSEvent) {
        editView.mouseDragged(theEvent)
    }

    func handleMouseUp(theEvent: NSEvent) {
        editView.mouseUp(theEvent)
    }
}

// AppWindowController.xib makes us the window's delegate (as nib owner), as well as its controler.
extension AppWindowController: NSWindowDelegate {
    func windowDidBecomeKey(notification: NSNotification) {
        editView.updateIsFrontmost(true)
    }
    func windowDidResignKey(notification: NSNotification) {
        editView.updateIsFrontmost(false);
        
    }
}
