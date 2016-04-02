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
    @IBOutlet weak var editView: EditView!
    @IBOutlet weak var scrollView: NSScrollView!
    @IBOutlet weak var shadowView: ShadowView!

    var coreConnection: CoreConnection?
    
    var filename: String?

    func visualConstraint(views: [String : NSView], _ format: String) {
        let constraints = NSLayoutConstraint.constraintsWithVisualFormat(format, options: .AlignAllTop, metrics: nil, views: views)
        NSLayoutConstraint.activateConstraints(constraints)
    }

    override func windowDidLoad() {
        super.windowDidLoad()
        //window?.backgroundColor = NSColor.whiteColor()
        editView.coreConnection = coreConnection

        // set up autolayout constraints
        let views = ["editView": editView, "clipView": scrollView.contentView]
        visualConstraint(views, "H:[editView(>=clipView)]")
        visualConstraint(views, "V:[editView(>=clipView)]")

        NSNotificationCenter.defaultCenter().addObserver(self, selector: "boundsDidChangeNotification:", name: NSViewBoundsDidChangeNotification, object: scrollView.contentView)
        NSNotificationCenter.defaultCenter().addObserver(self, selector: "frameDidChangeNotification:", name: NSViewFrameDidChangeNotification, object: scrollView)
        updateEditViewScroll()
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
        if filename == nil {
            saveDocumentAs(sender)
        } else {
            coreConnection?.sendJson(["save", filename!])
        }
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
}
